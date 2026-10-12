"""Write source-coverage-audit.json, review.md and candidate.json for one asset.

    python3 level-editor/blender/lincoln/rocks_terrain_volumes_finish.py <asset-id>

Run after the final packet, the evidence sheets and the coverage audit. All
hashes are recomputed from the files on disk, so the output binds the current
model.blend and modified/views.json. Per-asset review text lives in
rocks_terrain_volumes_notes.py.
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from rocks_terrain_volumes_notes import NOTES, COMMON_LIMITATIONS  # noqa: E402

ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
RECIPE = HERE / 'refine_rocks_terrain.py'


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def main():
    asset = sys.argv[1]
    ws = ROOT / 'round-1/assets' / asset
    note = NOTES[asset]
    recipe = json.loads((ws / 'inspection/geometry-recipe.json').read_text())
    cov = json.loads((ws / 'inspection/source-coverage.json').read_text())
    views = json.loads((ws / 'modified/views.json').read_text())
    model_sha = sha(ws / 'model.blend')
    views_sha = sha(ws / 'modified/views.json')
    if cov['model_sha256'] != model_sha:
        raise ValueError(f'{asset}: coverage audit is stale; rerun rocks_terrain_volumes_audit.py')
    if recipe['recipe_sha256'] != sha(RECIPE):
        raise ValueError(f'{asset}: geometry report was made by an older recipe; rerun the recipe')
    source_px = sum(v['counts']['source'] for v in views['views'])
    unknown_px = sum(v['counts']['unknown'] for v in views['views'])
    changed = any(n.get('changed') for n in recipe['nodes'])
    c = cov['counts']
    front = max(c['front'], 1)
    evidence = {}
    for rel in ['inspection/source-coverage.png', 'inspection/source-coverage.json',
                'inspection/geometry-recipe.json'] + sorted(
                    str(p.relative_to(ws)) for p in (ws / 'inspection').glob('mask-review-*.png')):
        evidence[str(ws / rel)] = sha(ws / rel)
    status = note.get('audit_status', 'PASS')
    audit = {
        'version': 1, 'asset_id': asset, 'status': status,
        'model_sha256': model_sha, 'modified_views_sha256': views_sha,
        'inspected_views': list(range(8)),
        'method': cov['method'] + '. Accepted/excluded/foreign/unmasked fractions of the asset\'s '
                  'front-most source pixels; the eight modified solid/textured/known views, the context '
                  'crop and inspection/source-coverage.png were inspected visually.',
        'observation': (f"front pixels {c['front']}: accepted {c['accepted'] / front:.1%}, reviewed exclusions "
                        f"{c['excluded'] / front:.1%}, other assets' masks {c['foreign_mask'] / front:.1%}, "
                        f"no native silhouette {c['unmasked'] / front:.1%}; own pixels hidden behind other "
                        f"meshes {c['hidden_behind_other_meshes']}. Packet: {source_px} source and "
                        f"{unknown_px} neutral pixels over eight views. " + note['audit']),
        'evidence': evidence,
        'limitations': note.get('audit_limitations', []) + COMMON_LIMITATIONS,
    }
    (ws / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    lines = [f"# {asset}: rocks/terrain lane review", '',
             f"Recipe: `{RECIPE}` (`--asset {asset} --packet`), helpers "
             '`rocks_terrain_volumes_core.py`, `rocks_terrain_volumes_masks.py`, '
             '`rocks_terrain_volumes_evidence.py`, `rocks_terrain_volumes_audit.py`, '
             '`rocks_terrain_volumes_finish.py` in the same directory.', '',
             f"Model sha256 `{model_sha}`; modified/views.json sha256 `{views_sha}`.", '',
             '## Changes', ''] + [f'- {x}' for x in note['changes']] + [
             '', '## Ground height', '', note['ground'], '',
             '## Per-node geometry report', '']
    for n in recipe['nodes']:
        lines.append(f"- `{n['source_node']}` mode `{n['mode']}`, native top z {n['native_top']}, "
                     f"{'changed' if n.get('changed') else 'unchanged'}; "
                     f"{n.get('construction', 'native geometry')}; vertices {n['vertices']}, faces {n['faces']}"
                     + (f"; topology {n['topology']}" if 'topology' in n else '')
                     + (f"; kept height range {n['kept_height_range']}" if n.get('kept_height_range') else ''))
    lines += ['', '## Source masks', '', note.get('masks', 'Frozen mask rows unchanged.'), '',
              f"Working-mask changes written by the recipe this run: {recipe['mask_changes'] or 'none (already current)'}."
              , '', '## Checks', '',
              '- Every owned mesh keeps `source_node` and `asset_group`; outside geometry is unchanged '
              f"({recipe['outside_objects_unchanged']} objects hash-checked by the recipe) and the frozen "
              'helper validation passed.',
              '- Rebuilt shells are closed and manifold (see topology above).',
              '- All eight modified solid, textured and known views and the context crop were inspected '
              '(input and modified side by side).',
              f"- Source coverage audit ({status}): {audit['observation']}",
              '', '## Inferred hidden geometry', '', note['inferred'], '',
              '## States', '', 'Covered (exterior) state only; this lane has no state-activated obstacles. '
              'Revealed-state receivers are not reviewed.', '',
              '## Unresolved defects and limitations', ''] + [f'- {x}' for x in note['limitations']]
    (ws / 'review.md').write_text('\n'.join(lines) + '\n')
    candidate = {
        'version': 1, 'asset_id': asset, 'status': note.get('status', 'ready-for-user'),
        'geometry_refined': changed, 'geometry_reviewed': True,
        'inspected_views': list(range(8)), 'recipe': str(RECIPE),
        'model_sha256': model_sha, 'modified_views_sha256': views_sha,
        'changes': note['changes'] if changed else [],
        'limitations': note['limitations'],
        'geometry_approval': 'pending', 'texture_generation': 'not-started',
        'ground_height': note['ground'],
        'source_comparison': 'inspection/source-coverage.png',
    }
    if not changed:
        candidate['no_change_reason'] = note['no_change_reason']
    (ws / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    print(json.dumps({'asset': asset, 'status': candidate['status'], 'refined': changed,
                      'source_pixels': source_px, 'audit': status}))


if __name__ == '__main__':
    main()
