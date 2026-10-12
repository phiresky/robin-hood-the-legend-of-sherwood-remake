"""Reviewed working-mask revisions for the Lincoln courtyard lane (pure Python).

Only rows of the workspace's own source nodes are changed, in the working
`source-masks.json` that `refinement_workspace` allows each asset to revise.
Every revision records its reason and an evidence image written to the
workspace's `inspection/mask-revisions.png` (artwork | own masks in cyan,
excluded foreground masks in magenta).

Foreground masks were chosen by listing every native mask overlapping the
asset's own masks and inspecting each over the covered artwork:
  68  bush in a tub at the shed's west corner (in front of wall and knee wall)
  121 rail fence in front of the shed's west bay and the hay cart
  173 hay heap inside the shed's west bay (in front of the back wall)
  174 goat inside the shed (in front of the back wall)
  176 manger plank inside the shed (in front of the back wall)
  67  bush against the thatched cottage's west gable
  123 low fence in front of the thatched cottage's west corner
  153 shrub drawn over the south lean-to's west roof edge
  157/161 tree foliage (duplicate layer-0/layer-2 masks) over the west hutch
  122 the stile (thatched-cottage asset) in front of the shingle cottage's
      west corner (added in round 2)
Parapet masks 187/190/191/192 overlap the south sheds but are already fully
covered by the reviewed curtain exclusions 230/231/232 (residual <= 4 px).
Masks 206/227 (inner gatehouse masonry) lie behind the privy and stay.
"""
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

LINCOLN = Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement'
BARN = 'the barn masks 172/175 cover the whole shed'
FOREGROUND = ('Visually reviewed foreground silhouettes drawn in front of this receiver in the covered '
              'artwork (foliage, fence, interior props or animals); those pixels belong to the foreground '
              'object and must not be painted on this surface.')

SHED_BARN = {'exclude_mask_indices': [68, 121, 173, 174, 176]}
REVISIONS = {
    'lincoln-courtyard-shed': {
        **{node: SHED_BARN for node in ('building-188', 'building-191', 'building-192', 'building-194',
                                        'building-195', 'building-224')},
        # 193 was reject-all while it was read as an unexplained pillar; it is
        # the roof's east verge board, drawn inside the barn roof silhouette.
        'building-193': {'mask_indices': [172, 175], 'exclude_mask_indices': [68, 121, 173, 174, 176],
                         'constraint_kind': 'reviewed-native-silhouette', 'native_ownership_reviewed': True,
                         'review_group': 'b-barn-172',
                         'review_note': ('Worker revision: obstacle 193 is the 6-unit verge board along the '
                                         'shed roof east edge (top follows the roof slope 358->302); '
                                         + BARN + ', including this edge.')},
    },
    'lincoln-bailey-thatched-cottage': {
        node: {'exclude_mask_indices': [67, 123]} for node in ('building-349', 'building-350', 'building-351')},
    # Round 2: the stile 355 (thatched cottage asset) is drawn in front of the
    # shingle cottage's west corner; keep the existing thatch exclusion 168.
    'lincoln-bailey-shingle-cottage': {
        node: {'exclude_mask_indices': [168, 122]} for node in ('building-352', 'building-353', 'building-354')},
    'lincoln-bailey-hay-cart': {
        node: {'exclude_mask_indices': [121]} for node in ('building-359', 'building-360', 'building-361')},
    'lincoln-south-wall-lean-to': {'building-368': {'exclude_mask_indices': [231, 153]}},
    'lincoln-northeast-yard-hutches': {'building-169': {'exclude_mask_indices': [157, 161]}},
}


def _manifest():
    return {e['global_index']: e for e in
            json.loads((LINCOLN / 'source-states/mask-manifest.json').read_text())['masks']}


def _evidence(workspace, rows):
    manifest = _manifest()
    source = Image.open(LINCOLN / 'source-states/covered.png').convert('RGB')
    own = sorted({i for r in rows for i in r['mask_indices']} - {428})
    excluded = sorted({i for r in rows for i in r.get('exclude_mask_indices', [])})
    boxes = [(*manifest[i]['box_top_left'], *manifest[i]['box_size']) for i in own]
    x0 = min(b[0] for b in boxes) - 30
    y0 = min(b[1] for b in boxes) - 30
    x1 = max(b[0] + b[2] for b in boxes) + 30
    y1 = max(b[1] + b[3] for b in boxes) + 30
    crop = np.array(source.crop((x0, y0, x1, y1))).astype(float)
    overlay = crop.copy()
    for indices, colour in ((own, (0, 255, 255)), (excluded, (255, 0, 255))):
        for i in indices:
            e = manifest[i]
            mk = np.array(Image.open(LINCOLN / 'source-states' / e['image'])) > 0
            bx, by = e['box_top_left']
            ys, xs = np.nonzero(mk)
            X, Y = xs + bx - x0, ys + by - y0
            keep = (X >= 0) & (X < crop.shape[1]) & (Y >= 0) & (Y < crop.shape[0])
            overlay[Y[keep], X[keep]] = overlay[Y[keep], X[keep]] * 0.45 + np.array(colour) * 0.55
    sheet = Image.new('RGB', (crop.shape[1] * 2, crop.shape[0] + 16))
    sheet.paste(Image.fromarray(crop.astype(np.uint8)), (0, 0))
    sheet.paste(Image.fromarray(overlay.astype(np.uint8)), (crop.shape[1], 0))
    ImageDraw.Draw(sheet).text((3, crop.shape[0] + 2), f'own {own} cyan | excluded {excluded} magenta | '
                               f'artwork box {x0},{y0}-{x1},{y1}', fill=(255, 255, 0))
    scale = max(1, 900 // sheet.width)
    sheet = sheet.resize((sheet.width * scale, sheet.height * scale), Image.NEAREST)
    path = workspace / 'inspection' / 'mask-revisions.png'
    path.parent.mkdir(exist_ok=True)
    sheet.save(path)
    return path


def apply(workspace):
    """Idempotently apply this asset's reviewed revisions; returns a report."""
    workspace = Path(workspace).resolve()
    config = json.loads((workspace / 'workspace.json').read_text())
    revisions = REVISIONS.get(config['asset_id'], {})
    if not revisions:
        return {'asset_id': config['asset_id'], 'revisions': {}}
    if set(revisions) - set(config['part_ids']):
        raise ValueError('Mask revision targets a node outside this asset')
    path = Path(config['source_mask_manifest'])
    working = json.loads(path.read_text())
    frozen = json.loads((Path(config['mask_reference']) / 'assignments.json').read_text())
    frozen_rows = {r['source_node']: r for r in frozen['projections']['exterior']['assignments']
                   if 'source_node' in r}
    rows = working['projections']['exterior']['assignments']
    evidence_rows = []
    for index, row in enumerate(rows):
        node = row.get('source_node')
        if node not in revisions:
            continue
        revised = dict(frozen_rows[node])  # rebuild from the frozen origin
        revised.update(revisions[node])
        revised['exclusions_reviewed'] = True
        revised['exclusion_reason'] = FOREGROUND
        revised['worker_revision'] = 'courtyard lane: reviewed foreground exclusions' + (
            ' and receiver assignment' if 'mask_indices' in revisions[node] else '')
        rows[index] = revised
        evidence_rows.append(revised)
    evidence = _evidence(workspace, evidence_rows)
    for row in evidence_rows:
        row['exclusion_evidence'] = str(evidence)
    path.write_text(json.dumps(working, indent=2) + '\n')
    return {'asset_id': config['asset_id'], 'evidence': str(evidence),
            'revisions': {r['source_node']: {k: r[k] for k in ('mask_indices', 'exclude_mask_indices')}
                          for r in evidence_rows}}
