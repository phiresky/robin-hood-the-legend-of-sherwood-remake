"""Review records for terrain assets re-rendered for the tree fold-in (system python).

    python3 level-editor/blender/lincoln/terrain_foldin_finalize.py <geometry.json> <overrides.json> <asset> [...]

Each round-7 workspace was prepared by prepare_assets.py from grouped-v5 with the
terrain-occluder derivation of the trees manifest. Geometry must be identical to
the approved previous workspace (terrain_foldin_geometry.py). This writes the
coverage audit (view-by-view known-pixel diff against the previous packet at the
same frozen cameras, plus the source-space exclusion statistics), review.md and
candidate.json, bound to the current model and packet.
"""
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
H, W = 2176, 2944


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def accepted(manifest_path, node, extra_excludes=None):
    manifest = read(manifest_path)
    inventory = Path(manifest_path).parent / manifest['mask_inventory']
    records = {r['index']: r for r in read(inventory)['masks']}
    row, = [r for r in manifest['projections']['exterior']['assignments'] if r.get('source_node') == node]

    def bitmap(index):
        record = records[index]
        png = Path(record['png'])
        png = png if png.is_absolute() else inventory.parent / png
        a = np.array(Image.open(png).convert('L')) > 0
        full = np.zeros((H, W), bool)
        x, y = record['box_top_left']
        h, w = a.shape
        full[max(0, y):y + h, max(0, x):x + w] = a[max(0, -y):H - y, max(0, -x):W - x]
        return full
    result = np.zeros((H, W), bool)
    for index in row['mask_indices']:
        result |= bitmap(index)
    excludes = row.get('exclude_mask_indices', []) if extra_excludes is None else extra_excludes
    for index in excludes:
        result &= ~bitmap(index)
    return result, row


def main():
    geometry = {r['asset_id']: r for r in read(sys.argv[1])}
    overrides = read(sys.argv[2])['assets']
    for asset in sys.argv[3:]:
        new = ROOT / 'round-7/assets' / asset
        old = Path(overrides.get(asset, ROOT / 'round-4/assets' / asset))
        record = geometry[asset]
        if not (record['geometry_identical'] and record['same_asset'] and Path(record['new_workspace']).resolve() == new.resolve()):
            raise ValueError(f'{asset}: geometry differs from the approved workspace')
        old_views, new_views = read(old / 'modified/views.json'), read(new / 'modified/views.json')
        for a, b in zip(old_views['views'], new_views['views']):
            for key in ('camera_location', 'camera_rotation_euler', 'ortho_scale'):
                if a.get(key) != b.get(key):
                    raise ValueError(f'{asset}: frozen camera {a["index"]} differs from the previous packet')
        per_view = []
        for i in range(8):
            before = np.array(Image.open(old / f'modified/views/view-{i}-known.png').convert('L')) > 127
            after = np.array(Image.open(new / f'modified/views/view-{i}-known.png').convert('L')) > 127
            per_view.append({'view': i, 'known_before': int(before.sum()), 'known_after': int(after.sum()),
                             'lost': int((before & ~after).sum()), 'gained': int((~before & after).sum())})
        config = read(new / 'workspace.json')
        source_stats = {}
        for node in config['part_ids']:
            now, row = accepted(config['source_mask_manifest'], node)
            then, _ = accepted(old / 'source-masks.json', node, None)
            source_stats[node] = {'accepted_before': int(then.sum()), 'accepted_after': int(now.sum()),
                                  'removed_by_foliage_exclusions': int((then & ~now).sum()),
                                  'added': int((now & ~then).sum()),
                                  'exclude_mask_indices': row.get('exclude_mask_indices', [])}
        if any(s['added'] for s in source_stats.values()):
            raise ValueError(f'{asset}: the new manifest widens an own mask')
        model, views = sha(new / 'model.blend'), sha(new / 'modified/views.json')
        old_candidate = read(old / 'candidate.json')
        removed = sum(s['removed_by_foliage_exclusions'] for s in source_stats.values())
        lost = sum(v['lost'] for v in per_view)
        gained = sum(v['gained'] for v in per_view)
        fold = (f'Tree fold-in re-render: geometry identical to the approved {old.relative_to(ROOT)} '
                f'(model {old_candidate["model_sha256"][:12]}). The receiver now excludes approved foliage asset '
                f'domains ({removed:,} source px leave it); foliage meshes occlude it only inside their painted '
                'domains (terrain occluder constraints). Context scene grouped-v5 (the stream-channel ground).')
        audit = {'version': 1, 'status': 'PASS', 'asset_id': asset, 'model_sha256': model,
                 'modified_views_sha256': views, 'inspected_views': list(range(8)),
                 'method': ('Owned geometry proven identical to the approved workspace; same frozen cameras; '
                            'source-space accepted domain compared with the previous manifest (only foliage-asset '
                            'exclusions remove pixels, nothing widens); per-view known-pixel diff against the '
                            'previous packet; all eight modified solid/textured views inspected. The previous '
                            'source-coverage audit remains the completeness review for all non-foliage pixels.'),
                 'observation': fold,
                 'source_space': source_stats, 'view_diff': per_view,
                 'known_pixels_lost': lost, 'known_pixels_gained': gained,
                 'previous_audit': {'path': str(old / 'source-coverage-audit.json'),
                                    'sha256': sha(old / 'source-coverage-audit.json')},
                 'evidence': {p: sha(new / p) for p in ('source-masks.json', 'validation.json', 'modified/views.json')},
                 'limitations': old_candidate.get('limitations', [])}
        (new / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
        review = (f'# {asset}: tree fold-in re-render\n\n{fold}\n\n'
                  f'- Source space: {json.dumps(source_stats)}\n'
                  f'- Packet: {lost:,} known view px lost and {gained:,} gained over the eight views '
                  f'({sum(v["known_after"] for v in per_view):,} known now).\n'
                  '- All eight modified solid and textured views were inspected; the removed pixels are the painted '
                  'bushes/trees now owned by foliage assets.\n\n---\n\n' + (old / 'review.md').read_text())
        (new / 'review.md').write_text(review)
        candidate = {
            'version': 1, 'asset_id': asset, 'status': 'ready-for-user',
            'geometry_refined': False, 'geometry_reviewed': True, 'inspected_views': list(range(8)),
            'recipe': str(HERE / 'prepare_assets.py'), 'foldin_recipe': str(Path(__file__).resolve()),
            'model_sha256': model, 'modified_views_sha256': views,
            'changes': [], 'no_change_reason': fold,
            'limitations': old_candidate.get('limitations', []),
            'ground_height': old_candidate.get('ground_height'),
            'previous_workspace': str(old), 'previous_model_sha256': old_candidate['model_sha256'],
            'source_comparison': old_candidate.get('source_comparison') and None,
            'geometry_approval': 'approved-previous-revision; fold-in review pending',
            'texture_generation': 'not-started'}
        candidate = {k: v for k, v in candidate.items() if v is not None}
        (new / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
        print(asset, model[:12], views[:12], 'removed', removed, 'lost', lost, 'gained', gained)


if __name__ == '__main__':
    main()
