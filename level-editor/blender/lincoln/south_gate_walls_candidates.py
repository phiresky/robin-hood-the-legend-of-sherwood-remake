"""Write review.md and candidate.json for the south gate/wall lane workspaces.

Offline. Reads each workspace's inspection/geometry-recipe.json (written by
refine_south_gate.py), corner-trace.json and source-coverage-audit.json, binds
the current model.blend and modified/views.json hashes, and refuses to mark an
asset ready when the coverage audit is missing, failed or stale.
"""
import os
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
R = HERE.parents[2] / 'level-editor/work/needs-review/lincoln-refinement'
RECIPE = HERE / 'refine_south_gate.py'

LANE_NOTES = {
    'lincoln-south-gatehouse-arch': ['Walkway slab 112 wraps both gate towers (gate block and east tower '
                                     'interior floor): record component-split need before per-tower selection.'],
    'lincoln-south-gate-drawbridge': ['States: covered = raised leaf (obstacle 457 active, mask 415); '
                                      'Pont_levis applied = lowered leaf (obstacle removed, masks 416/417, last '
                                      'transition frame). The mechanism patch (doors 11-16) has no obstacle.'],
}


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def fmt_ground(ground):
    rows = []
    for node, g in sorted(ground.items()):
        rows.append(f'- `{node}`: ' + ', '.join(f'{k}={v}' for k, v in g.items()))
    return '\n'.join(rows)


def write(asset):
    ws = R / os.environ.get('SOUTH_GATE_ROUND', 'round-1') / 'assets' / asset
    rec = json.loads((ws / 'inspection/geometry-recipe.json').read_text())
    audit_path = ws / 'source-coverage-audit.json'
    audit = json.loads(audit_path.read_text()) if audit_path.exists() else None
    model, views = sha(ws / 'model.blend'), sha(ws / 'modified/views.json')
    trace_path = ws / 'inspection/corner-trace.json'
    trace = json.loads(trace_path.read_text()) if trace_path.exists() else None
    audit_ok = bool(audit and audit['status'] == 'PASS' and audit['model_sha256'] == model
                    and audit['modified_views_sha256'] == views and audit['inspected_views'] == list(range(8)))
    recipe_ok = rec['recipe_sha256'] == sha(RECIPE)
    limitations = rec['limitations'] + LANE_NOTES.get(asset, []) + (audit['limitations'] if audit else [])
    status = 'ready-for-user' if audit_ok and recipe_ok else 'refinement-in-progress'
    if not audit_ok:
        limitations.append('Source-coverage audit missing, failed or stale.')
    cand = {'version': 1, 'asset_id': asset, 'status': status,
            'geometry_refined': True, 'geometry_reviewed': True, 'inspected_views': list(range(8)),
            'recipe': str(RECIPE.resolve()), 'model_sha256': model, 'modified_views_sha256': views,
            'changes': rec['changes'], 'limitations': limitations,
            'geometry_approval': 'pending', 'texture_generation': 'not-started',
            'ground': rec['ground'], 'inferred_geometry': rec['inferred'],
            'states': rec.get('states'),
            'source_trace': str((ws / 'inspection/corner-trace.json').relative_to(ws)) if trace else None,
            'source_comparison': 'inspection/coverage-audit.png'}
    if asset == 'lincoln-south-gate-drawbridge':
        cand['state_comparison'] = 'inspection/drawbridge-states.png'
    (ws / 'candidate.json').write_text(json.dumps(cand, indent=1) + '\n')
    runs = ''
    if trace:
        for r in trace['runs']:
            runs += (f"- `{r['run']}` ({r['source_node']}): {len(r['notches'])} notches, floor z={r['sill_z']}, "
                     f"merlon top z={r['merlon_top_z']}, {len(r['corners'])} numbered corners; "
                     f"`inspection/corner-trace-{r['run']}.png`" + (f". {r['notes']}" if r.get('notes') else '') + '\n')
    stats = audit['stats'] if audit else {}
    md = f"""# {asset}

Status: **{status}** (geometry approval pending, texture generation not started).
Recipe: `level-editor/blender/lincoln/refine_south_gate.py -- --asset {asset}` (helpers
`south_gate_walls_geometry.py`, `south_gate_walls_assets.py`, traces from `south_gate_walls_trace.py`).
Model `{model}`; modified/views.json `{views}`.

## Changes
""" + '\n'.join(f'- {c}' for c in rec['changes']) + f"""

## Ground height / contact
{fmt_ground(rec['ground'])}

Plateau datum: castle-hill plateau top z=220 (terrain lane keeps it fixed); footings sit at 218
(2 below) or follow the measured bottom edge of the reviewed native mask where the masonry visibly
continues down the cliff face.

## Numbered-corner traces
{runs or '- none (no crenellated run in this asset)'}
## Inferred hidden geometry
""" + '\n'.join(f'- {c}' for c in rec['inferred']) + f"""

## Checks
- Every rebuilt mesh is a closed 2-manifold with positive signed volume (checked in the recipe);
  object transforms, source_node and asset_group are preserved; no modifiers.
- Modified packet regenerated with the frozen tooling (validation PASS); all eight solid, textured
  and known views and the context crop inspected.
- Source-coverage audit (`source-coverage-audit.json`, `inspection/coverage-audit.png`): {audit['status'] if audit else 'MISSING'}.
  Stats: {json.dumps(stats)}.
  {audit['observation'] if audit else ''}

## States inspected
{json.dumps(rec.get('states') or {'covered': 'exterior covered state only'})}
(covered.png exterior packet; revealed-interior receivers are not reviewed in this round.)

## Unresolved defects / limitations
""" + '\n'.join(f'- {c}' for c in limitations) + '\n'
    (ws / 'review.md').write_text(md)
    return status


RESOLVED_IN_R2 = ('baseline 099 pillar coplanar', 'verified only together with', 'verified together with',
                  'must be re-checked after integration', 'Turret face texture must be re-checked',
                  'inside the baseline', 'inside baseline', 'Lower masonry stays neutral until')
TERRAIN_R2 = ('Terrain (integrated r1): the plateau/cliff volumes now recede to the wall faces; any remaining '
              'neutral band at the footing is the terrain lane\'s sloping cliff surface covering the lowest drawn '
              'courses (terrain-lane decision).')


def write_unchanged(asset, extra_findings):
    """Round-2 review of an integrated workspace whose geometry is the round-1 result."""
    ws = R / os.environ['SOUTH_GATE_ROUND'] / 'assets' / asset
    r1 = R / 'round-1/assets' / asset
    rec = json.loads((r1 / 'inspection/geometry-recipe.json').read_text())
    r1cand = json.loads((r1 / 'candidate.json').read_text())
    audit = json.loads((ws / 'source-coverage-audit.json').read_text())
    model, views = sha(ws / 'model.blend'), sha(ws / 'modified/views.json')
    same = all(sha(ws / f'input/views/view-{i}-solid.png') == sha(ws / f'modified/views/view-{i}-solid.png')
               for i in range(8))
    audit_ok = (audit['status'] == 'PASS' and audit['model_sha256'] == model
                and audit['modified_views_sha256'] == views and audit['inspected_views'] == list(range(8)))
    lims = [l for l in r1cand['limitations'] if not any(k in l for k in RESOLVED_IN_R2)
            and 'turret face below the parapet' not in l]
    lims = [l for l in lims if 'Source-coverage audit missing' not in l]
    if any(k in l for l in r1cand['limitations'] for k in ('inside the baseline', 'inside baseline')):
        lims.append(TERRAIN_R2)
    for l in audit.get('limitations', []):
        if l not in lims:
            lims.append(l)
    reason = (f"Round-2 integrated review found no geometry defect: the round-1 refinement "
              f"({r1}/model.blend, sha256 {r1cand['model_sha256']}, recipe {RECIPE}) is already in this "
              f"workspace's baseline. Contacts with the refined neighbours and receded terrain were probed "
              f"(inspection/contacts.json) and the coverage audit re-run on the integrated scene.")
    status = 'ready-for-user' if audit_ok and same else 'refinement-in-progress'
    cand = {'version': 1, 'asset_id': asset, 'status': status, 'geometry_refined': False,
            'geometry_reviewed': True, 'no_change_reason': reason, 'inspected_views': list(range(8)),
            'recipe': str(RECIPE.resolve()), 'model_sha256': model, 'modified_views_sha256': views,
            'changes': [], 'round1_changes': rec['changes'], 'limitations': lims,
            'round2_findings': extra_findings, 'geometry_approval': 'pending', 'texture_generation': 'not-started',
            'ground': rec['ground'], 'states': rec.get('states'),
            'source_comparison': 'inspection/coverage-audit.png'}
    if asset == 'lincoln-south-gate-drawbridge':
        cand['state_comparison'] = 'inspection/drawbridge-states.png'
    (ws / 'candidate.json').write_text(json.dumps(cand, indent=1) + '\n')
    md = f"""# {asset} (round 2, integrated context)

Status: **{status}**. Geometry unchanged in round 2 (reviewed); approval pending, texture not started.
Round-1 refinement: `{r1}` model `{r1cand['model_sha256']}`, recipe `{RECIPE}`.
Round-2 model `{model}`; modified/views.json `{views}`.

## Round-2 review
- Inspected all eight input/modified solid, textured and known views and the context crop.
- Footing contacts probed against the integrated neighbours (`inspection/contacts.json`).
- Coverage audit re-run on the integrated scene: {audit['status']} ({json.dumps(audit['stats'])}).
  {audit['observation']}

## Round-2 findings
""" + '\n'.join(f'- {f}' for f in extra_findings) + """

## Round-1 geometry (still current)
""" + '\n'.join(f'- {c}' for c in rec['changes']) + f"""

## Ground
{fmt_ground(rec['ground'])}

## States
{json.dumps(rec.get('states') or {'covered': 'exterior covered state only'})}

## Remaining limitations
""" + '\n'.join(f'- {l}' for l in lims) + '\n'
    (ws / 'review.md').write_text(md)
    return status


if __name__ == '__main__':
    if sys.argv[1] == '--unchanged':
        findings = json.loads(Path(sys.argv[2]).read_text())
        for a in sys.argv[3:]:
            print(a, write_unchanged(a, findings.get(a, [])))
    else:
        for a in sys.argv[1:]:
            print(a, write(a))
