"""Write hash-bound candidate.json and source-coverage-audit.json for a workspace.

Plain Python (no Blender). Run after the final recipe packet, closeup and
coverage audit, once review.md and inspection/review-notes.json are written:

    python3 level-editor/blender/lincoln/west_complex_finalize.py --asset <id>

review-notes.json fields:
  status            ready-for-user | refinement-in-progress | fix-needed
  audit_status      PASS | FAIL
  audit_observation text explaining accepted/rejected/neutral areas
  audit_limitations [text]
  extra_limitations [text]   (appended to the recipe limitations)
  extra_changes     [text]   (optional)
The recipe report inspection/geometry-recipe.json supplies changes/limitations.
"""
import argparse
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Workspace round: round-1 (default) or round-2 (integrated context); set with
# --round or the WEST_COMPLEX_ROUND environment variable.
import os as _os
ROUND = int(_os.environ.get('WEST_COMPLEX_ROUND', '1'))
ASSETS_DIR = HERE.parents[1] / 'work/needs-review/lincoln-refinement' / f'round-{ROUND}/assets'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def finalize(asset):
    workspace = ASSETS_DIR / asset
    notes = json.loads((workspace / 'inspection/review-notes.json').read_text())
    report_path = workspace / 'inspection/geometry-recipe.json'
    unchanged = bool(notes.get('no_change_reason'))
    recipe_report = ({} if unchanged or not report_path.exists() else json.loads(report_path.read_text()))
    recipe_report.setdefault('module', notes.get('module'))
    coverage = json.loads((workspace / 'inspection/coverage/coverage.json').read_text())
    model = sha(workspace / 'model.blend')
    views = sha(workspace / 'modified/views.json')
    if coverage['model_sha256'] != model or coverage['modified_views_sha256'] != views:
        raise ValueError('Coverage audit is stale; rerun west_complex_audit.py after the final packet')
    if not (workspace / 'review.md').read_text().strip():
        raise ValueError('review.md is empty')
    evidence_files = ['inspection/coverage/coverage.png', 'inspection/coverage/coverage.json',
                      'inspection/closeup/textured.png', 'inspection/closeup/solid.png',
                      'inspection/closeup/context.png', 'modified/textured.png', 'modified/solid.png',
                      'modified/context.png', 'inspection/geometry-recipe.json']
    evidence_files += notes.get('extra_evidence', [])
    evidence = {str(workspace / p): sha(workspace / p) for p in evidence_files if (workspace / p).exists()}
    audit = {'version': 1, 'asset_id': asset, 'status': notes['audit_status'],
             'model_sha256': model, 'modified_views_sha256': views,
             'inspected_views': list(range(8)),
             'method': ('Independent ray-cast domain: every source pixel in the asset crop is cast from the '
                        '35-degree source camera through the saved model (all render-visible scene meshes). '
                        'Pixels whose first hit is an owned mesh form the geometry-visible domain; it is compared '
                        'against each owned node\'s reviewed native mask (union minus exclusions) from '
                        'source-masks.json. Categories: accepted, neutral-reject-all, neutral-outside-mask, '
                        'mask-foreign-hit (mask pixel first hit by another asset), mask-no-geometry. The '
                        'annotated crop (coverage.png) was inspected beside the unmarked artwork, and the saved '
                        'materials were inspected in the frozen modified packet and the tight closeup packet '
                        '(view 0 is the source camera).'),
             'counts': coverage['counts'], 'per_node': coverage['per_node'],
             'foreign_first_hits': coverage['foreign_first_hits'],
             'observation': notes['audit_observation'],
             'evidence': evidence,
             'limitations': notes.get('audit_limitations', [])}
    (workspace / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    candidate = {'version': 1, 'asset_id': asset, 'status': notes['status'],
                 'geometry_refined': not unchanged, 'geometry_reviewed': True,
                 'inspected_views': list(range(8)),
                 'recipe': str((HERE / 'refine_west_complex.py').resolve()),
                 'recipe_modules': {m: sha(HERE / m) for m in
                                    ('refine_west_complex.py', 'west_complex_geom.py', recipe_report['module'] + '.py')},
                 'model_sha256': model, 'modified_views_sha256': views,
                 'ground_native_z': recipe_report.get('ground_native_z', notes.get('ground_native_z')),
                 'changes': [] if unchanged else recipe_report.get('changes', []) + notes.get('extra_changes', []),
                 'limitations': (notes.get('limitations') if unchanged else recipe_report.get('limitations', []))
                                + notes.get('extra_limitations', []),
                 'geometry_approval': 'pending', 'texture_generation': 'not-started',
                 'source_comparison': 'inspection/closeup/textured.png',
                 'source_trace': notes.get('source_trace', 'inspection/coverage/coverage.png'),
                 'coverage_audit': 'source-coverage-audit.json'}
    if unchanged:
        candidate['no_change_reason'] = notes['no_change_reason']
    if not (workspace / 'inspection/closeup/textured.png').exists():
        candidate['source_comparison'] = 'modified/textured.png'
    if notes.get('projection_errors'):
        candidate['projection_errors'] = notes['projection_errors']
    (workspace / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    print(json.dumps({'asset': asset, 'status': candidate['status'], 'audit': audit['status'],
                      'model_sha256': model, 'modified_views_sha256': views}, indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--round', type=int, default=None)
    parser.add_argument('--asset', required=True)
    args = parser.parse_args()
    if args.round is not None:
        global ASSETS_DIR
        ASSETS_DIR = ASSETS_DIR.parents[1] / f'round-{args.round}/assets'
    finalize(args.asset)


if __name__ == '__main__':
    main()
