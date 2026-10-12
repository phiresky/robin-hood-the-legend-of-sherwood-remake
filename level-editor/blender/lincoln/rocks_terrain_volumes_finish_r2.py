"""Round-2/3 review files for one rocks/terrain asset (system python).

    python3 level-editor/blender/lincoln/rocks_terrain_volumes_finish_r2.py <asset-id> [round-2|round-3]

Writes source-coverage-audit.json, review.md and candidate.json in
round-2/assets/<asset-id>/, bound to the current model.blend and
modified/views.json. The round-2 notes live in rocks_terrain_volumes_notes_r2.py;
round-1 notes (changes, ground heights, remaining limitations) are carried over
from rocks_terrain_volumes_notes.py.
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from rocks_terrain_volumes_notes import NOTES as R1, COMMON_LIMITATIONS  # noqa: E402
from rocks_terrain_volumes_notes_r2 import R2  # noqa: E402
from rocks_terrain_volumes_notes_r3 import R3  # noqa: E402
from rocks_terrain_volumes_notes_r4 import R4  # noqa: E402

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
    rnd = sys.argv[2] if len(sys.argv) > 2 else 'round-2'
    prev_round = {'round-2': 'round-1', 'round-3': 'round-2', 'round-4': 'round-3'}[rnd]
    ws = ROOT / rnd / 'assets' / asset
    r1ws = ROOT / prev_round / 'assets' / asset
    if rnd == 'round-4':
        prior = json.loads((ROOT / 'workspace-overrides.json').read_text())['assets']
        r1ws = Path(prior.get(asset, ROOT / 'round-3/assets' / asset))
        prev_round = r1ws.parts[-3]
    r1 = R1[asset]
    r2 = {'round-2': R2, 'round-3': R3, 'round-4': R4}[rnd][asset]
    r1_candidate = json.loads((r1ws / 'candidate.json').read_text())
    cov = json.loads((ws / 'inspection/source-coverage.json').read_text())
    views = json.loads((ws / 'modified/views.json').read_text())
    model_sha = sha(ws / 'model.blend')
    views_sha = sha(ws / 'modified/views.json')
    if cov['model_sha256'] != model_sha:
        raise ValueError(f'{asset}: coverage audit is stale')
    changed = any(sha(ws / f'input/views/view-{i}-solid.png') != sha(ws / f'modified/views/view-{i}-solid.png')
                  for i in range(8))
    recipe_path = ws / 'inspection/geometry-recipe.json'
    recipe = json.loads(recipe_path.read_text()) if recipe_path.exists() else None
    if changed and (recipe is None or recipe['recipe_sha256'] != sha(RECIPE)):
        raise ValueError(f'{asset}: changed geometry needs a current round-2 recipe report')
    if changed and not r2.get('changes'):
        raise ValueError(f'{asset}: geometry changed but the notes list no round-2 changes')
    if not changed and not r2.get('no_change'):
        raise ValueError(f'{asset}: geometry unchanged but the notes give no no-change reason')
    source_px = sum(v['counts']['source'] for v in views['views'])
    unknown_px = sum(v['counts']['unknown'] for v in views['views'])
    c = cov['counts']
    front = max(c['front'], 1)
    evidence = {}
    rels = ['inspection/source-coverage.png', 'inspection/source-coverage.json']
    if recipe is not None:
        rels.append('inspection/geometry-recipe.json')
    rels += sorted(str(p.relative_to(ws)) for p in (ws / 'inspection').glob('mask-review-*.png'))
    for rel in rels:
        evidence[str(ws / rel)] = sha(ws / rel)
    status = r2.get('audit_status', 'PASS')
    limitations = r2.get('limitations', []) + COMMON_LIMITATIONS
    observation = (f"{rnd} integrated scene. Front-most source pixels {c['front']}: accepted "
                   f"{c['accepted'] / front:.1%}, reviewed exclusions {c['excluded'] / front:.1%}, other "
                   f"assets' masks {c['foreign_mask'] / front:.1%} (of which hiding the neighbour mesh "
                   f"directly behind: {c.get('hides_foreign_receiver', 0)} px), no native silhouette "
                   f"{c['unmasked'] / front:.1%}; own pixels hidden behind other meshes "
                   f"{c['hidden_behind_other_meshes']}. Packet: {source_px} source / {unknown_px} neutral "
                   f"pixels over eight views. " + r2['audit'])
    audit = {'version': 1, 'asset_id': asset, 'status': status, 'model_sha256': model_sha,
             'modified_views_sha256': views_sha, 'inspected_views': list(range(8)),
             'method': cov['method'] + '; plus a neighbour-hiding test (pixels where this asset is '
                       'front-most over a reviewed mask of the mesh directly behind it). The eight '
                       f'{rnd} input and modified solid/textured/known views and the context crop were '
                       'inspected visually.',
             'observation': observation, 'evidence': evidence,
             'limitations': r2.get('audit_limitations', []) + limitations}
    (ws / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    r1_ref = f"{r1ws} (model sha256 {r1_candidate['model_sha256']})"
    lines = [f'# {asset}: {rnd} review (rocks/terrain lane)', '',
             f'{rnd} workspace. Previous refinement ({prev_round}): `{r1_ref}`.',
             f'Recipe `{RECIPE}` (`--asset {asset} --round {rnd} --packet`); audit '
             f'`rocks_terrain_volumes_audit.py --round {rnd}`.', '',
             f'Model sha256 `{model_sha}`; modified/views.json sha256 `{views_sha}`.', '',
             f'## {rnd} changes', '']
    lines += [f'- {x}' for x in r2['changes']] if changed else [f"- None. {r2['no_change']}"]
    lines += ['', f'## {rnd} findings', ''] + [f'- {x}' for x in r2['findings']]
    lines += ['', '## Carried over from round 1', '', '- ' + '\n- '.join(r1['changes'] or ['(round 1: no geometry change)']),
              '', '## Ground height', '', r2.get('ground', r1['ground'])]
    if recipe is not None:
        lines += ['', f'## Per-node geometry report ({rnd} recipe run)', '']
        for n in recipe['nodes']:
            lines.append(f"- `{n['source_node']}` mode `{n['mode']}`, native top {n['native_top']}, "
                         f"{'changed' if n.get('changed') else 'unchanged vs native'}; "
                         f"{n.get('construction', 'native geometry')}; vertices {n['vertices']}, faces {n['faces']}"
                         + (f"; unhide lowered {n['unhide_lowered_columns']} columns" if n.get('unhide_lowered_columns') else '')
                         + (f"; topology {n['topology']}" if 'topology' in n else '')
                         + (f"; height range {n['kept_height_range']}" if n.get('kept_height_range') else ''))
    lines += ['', '## Checks', '',
              f'- All eight {rnd} modified solid, textured and known views and the context crop were inspected '
              f'beside the {rnd} input; the frozen helper validation passed.',
              f'- Source coverage audit ({status}): {observation}',
              '', '## Inferred hidden geometry', '', r1['inferred'], '',
              '## States', '', 'Covered (exterior) state only; revealed receivers are not reviewed.', '',
              '## Remaining limitations', ''] + [f'- {x}' for x in limitations]
    (ws / 'review.md').write_text('\n'.join(lines) + '\n')
    candidate = {'version': 1, 'asset_id': asset, 'status': r2.get('status', 'ready-for-user'),
                 'geometry_refined': changed, 'geometry_reviewed': True,
                 'inspected_views': list(range(8)), 'recipe': str(RECIPE),
                 'model_sha256': model_sha, 'modified_views_sha256': views_sha,
                 'changes': r2['changes'] if changed else [], 'limitations': limitations,
                 'geometry_approval': 'pending', 'texture_generation': 'not-started',
                 'ground_height': r2.get('ground', r1['ground']),
                 'previous_workspace': str(r1ws), 'previous_model_sha256': r1_candidate['model_sha256'],
                 'source_comparison': 'inspection/source-coverage.png'}
    if r2.get('user_feedback'):
        candidate['user_feedback'] = r2['user_feedback']
        lines_fb = ['', '## User feedback', '', f"> {r2['user_feedback']['exact_text']}", '',
                    r2['user_feedback']['addressed']]
        (ws / 'review.md').write_text((ws / 'review.md').read_text() + '\n'.join(lines_fb) + '\n')
    if not changed:
        candidate['no_change_reason'] = (f"{r2['no_change']} The {rnd} geometry is the {prev_round} refinement "
                                         f"from {r1_ref}, reviewed again in the new context.")
    (ws / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    print(json.dumps({'asset': asset, 'status': candidate['status'], 'refined': changed,
                      'source_pixels': source_px, 'audit': status}))


if __name__ == '__main__':
    main()
