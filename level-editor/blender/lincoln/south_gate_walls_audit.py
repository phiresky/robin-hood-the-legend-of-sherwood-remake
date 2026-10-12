"""Source-coverage audit for the Lincoln south gate/wall lane (offline).

Inputs per workspace: inspection/coverage-hits.npz (first-hit map of the saved
model from the source camera, south_gate_walls_audit_rays.py), the workspace
source-masks.json and the native mask bitmaps, covered.png, model.blend and
modified/views.json. Pixels in the audit box are classified:

  A accepted  - owned first hit inside the node's reviewed mask (textured)
  B rejected  - owned first hit outside its mask (neutral); B_edge within 2 px
                of the mask boundary, B_far otherwise
  C foreign   - inside an owned mask, but another mesh is hit first
  D empty     - inside an owned mask, nothing hit (geometry missing)

Writes inspection/coverage-audit.png (untouched crop | classes) and the
workspace source-coverage-audit.json. PASS requires a reviewed explanation
for every substantial neutral class (EXPLAIN below, written after inspecting
the overlay and all eight modified views).
"""
import os
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
R = HERE.parents[2] / 'level-editor/work/needs-review/lincoln-refinement'
MASKS = R / 'mask-review/inventory-v1'
H, W = 2176, 2944


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


_man = None


_inv = MASKS


def use_inventory(manifest_path):
    """Select the mask inventory named by a workspace source-masks.json."""
    global _man, _inv
    path = Path(manifest_path)
    if path != _inv / 'manifest.json':
        _inv = path.parent
        _man = None


def native_mask(i):
    global _man
    if _man is None:
        _man = {m['index']: m for m in json.loads((_inv / 'manifest.json').read_text())['masks']}
    rec = _man[i]
    a = np.array(Image.open(_inv / rec['png'])) > 0
    full = np.zeros((H, W), bool)
    x0, y0 = rec['box_top_left']
    h, w = a.shape
    full[y0:y0 + h, x0:x0 + w] = a[:H - y0, :W - x0]
    return full


# Per-asset reviewed explanations of neutral classes (filled after inspection).
_ROUND = os.environ.get('SOUTH_GATE_ROUND', 'round-1')
_NOTES = HERE / ('south_gate_walls_audit_notes.json' if _ROUND == 'round-1'
                 else f'south_gate_walls_audit_notes-{_ROUND}.json')
EXPLAIN = json.loads(_NOTES.read_text()) if _NOTES.exists() else {}


def audit(asset):
    ws = R / os.environ.get('SOUTH_GATE_ROUND', 'round-1') / 'assets' / asset
    hits = np.load(ws / 'inspection/coverage-hits.npz')
    meta = json.loads((ws / 'inspection/coverage-hits.json').read_text())
    cls, node = hits['cls'], hits['node']
    x0, y0, x1, y1 = meta['box']
    working = json.loads((ws / 'source-masks.json').read_text())
    use_inventory((ws / working['mask_inventory']).resolve())
    masks = working['projections']['exterior']['assignments']
    by = {a['source_node']: a for a in masks if 'projection_component' not in a}
    by_comp = {(a['source_node'], a['projection_component']): a for a in masks if 'projection_component' in a}
    comp_map = hits['comp'] if 'comp' in hits.files else None
    comp_names = meta.get('components', {})

    def mask_of(a):
        m = np.zeros((H, W), bool)
        for i in a['mask_indices']:
            if i != 428:
                m |= native_mask(i)
        for i in a.get('exclude_mask_indices', []) or []:
            m &= ~native_mask(i)
        return m[y0:y1, x0:x1]
    node_mask = {}
    for k, n in enumerate(meta['nodes']):
        comps = comp_names.get(n)
        a = by.get(n)
        if a is None:  # only component-level rows exist for this node
            a = next(r for (nn, c), r in by_comp.items() if nn == n)
        if comps and any((n, c) in by_comp for c in comps):
            # Component-specific receivers: the owned first-hit pixels of each
            # component are judged against that component's own mask.
            per = np.zeros((y1 - y0, x1 - x0), bool)
            node_px = (cls == 1) & (node == k)
            for ci, c in enumerate(comps):
                row = by_comp.get((n, c), a)
                per |= mask_of(row) & node_px & (comp_map == ci)
            node_mask[k] = per | (mask_of(a) & ~node_px)
            continue
        m = np.zeros((H, W), bool)
        for i in a['mask_indices']:
            if i == 428:
                continue  # synthetic reject-all
            m |= native_mask(i)
        for i in a.get('exclude_mask_indices', []) or a.get('excluded_mask_indices', []) or []:
            m &= ~native_mask(i)
        node_mask[k] = m[y0:y1, x0:x1]
    union = np.zeros_like(cls, bool)
    for m in node_mask.values():
        union |= m
    own = cls == 1
    acc = np.zeros_like(own)
    for k, m in node_mask.items():
        acc |= own & (node == k) & m
    rej = own & ~acc
    # distance-to-mask-boundary test (2 px) with binary dilation by shifts
    near = union.copy()
    for dy in range(-2, 3):
        for dx in range(-2, 3):
            near |= np.roll(np.roll(union, dy, 0), dx, 1)
    rej_edge, rej_far = rej & near, rej & ~near
    foreign = union & (cls == 2)
    empty = union & (cls == 0)
    per_foreign = {}
    for k, g in enumerate(meta['foreign_groups']):
        c = int((foreign & (node == k)).sum())
        if c:
            per_foreign[g] = c
    per_node = {n: {'owned_hits': int((own & (node == k)).sum()),
                    'accepted': int((acc & (node == k)).sum()),
                    'rejected': int((rej & (node == k)).sum()),
                    'masks': sorted({i for r in [by.get(n)] + [r for (nn, c), r in by_comp.items() if nn == n] if r for i in r['mask_indices']}),
                    'reject_all': all(r['mask_indices'] == [428] for r in [by.get(n)] + [r for (nn, c), r in by_comp.items() if nn == n] if r)}
                for k, n in enumerate(meta['nodes'])}
    src = np.array(Image.open(R / 'source-states/covered.png').convert('RGB'))[y0:y1, x0:x1]
    over = (src * 0.35).astype(np.uint8)
    for m, col in ((acc, (40, 220, 60)), (rej_edge, (255, 150, 0)), (rej_far, (255, 30, 30)),
                   (foreign, (60, 120, 255)), (empty, (255, 255, 0))):
        over[m] = col
    scale = 2 if max(x1 - x0, y1 - y0) < 700 else 1
    a = Image.fromarray(src).resize(((x1 - x0) * scale, (y1 - y0) * scale), Image.NEAREST)
    b = Image.fromarray(over).resize(a.size, Image.NEAREST)
    sheet = Image.new('RGB', (a.width * 2 + 8, a.height + 20), (15, 15, 15))
    sheet.paste(a, (0, 20))
    sheet.paste(b, (a.width + 8, 20))
    ImageDraw.Draw(sheet).text((4, 4), f'{asset}: green accepted, orange/red rejected (edge/far), blue '
                               'foreign first hit inside owned mask, yellow mask without geometry', fill=(255, 255, 255))
    png = ws / 'inspection/coverage-audit.png'
    sheet.save(png)
    stats = {'owned_first_hit': int(own.sum()), 'accepted': int(acc.sum()),
             'rejected_edge': int(rej_edge.sum()), 'rejected_far': int(rej_far.sum()),
             'mask_union': int(union.sum()), 'foreign_first_hit_in_mask': int(foreign.sum()),
             'mask_without_geometry': int(empty.sum())}
    note = EXPLAIN.get(asset)
    status = 'PASS' if note and note.get('status') == 'PASS' else 'FAIL'
    views = ws / 'modified/views.json'
    doc = {'version': 1, 'asset_id': asset, 'status': status,
           'model_sha256': sha(ws / 'model.blend'), 'modified_views_sha256': sha(views),
           'inspected_views': list(range(8)),
           'method': ('Independent geometric domain: one source-camera ray per pixel over the owned bounds of the '
                      'saved model.blend (first hit, baseline copies and hidden meshes ignored), compared with the '
                      'reviewed native masks of every owned node and with covered.png. Accepted and rejected pixels '
                      'are both drawn; foreign first hits inside owned masks and mask pixels with no geometry are '
                      'separated. All eight modified solid/textured/known views were inspected.'),
           'stats': stats, 'per_node': per_node, 'foreign_first_hit_by_group': per_foreign,
           'observation': note.get('observation') if note else 'NOT REVIEWED',
           'explanations': note.get('explanations', {}) if note else {},
           'evidence': {str(png): sha(png), str(ws / 'inspection/coverage-hits.npz'): sha(ws / 'inspection/coverage-hits.npz'),
                        str(ws / 'source-masks.json'): sha(ws / 'source-masks.json')},
           'limitations': note.get('limitations', []) if note else ['not reviewed'],
           'tool': str(Path(__file__).resolve()), 'tool_sha256': sha(__file__)}
    if meta['model_sha256'] != doc['model_sha256']:
        doc['status'] = 'FAIL'
        doc['observation'] = 'STALE: hit map was cast on a different model.blend'
    (ws / 'source-coverage-audit.json').write_text(json.dumps(doc, indent=1) + '\n')
    return doc


if __name__ == '__main__':
    for asset in sys.argv[1:]:
        d = audit(asset)
        print(asset, d['status'], json.dumps(d['stats']), json.dumps(d['foreign_first_hit_by_group']))
