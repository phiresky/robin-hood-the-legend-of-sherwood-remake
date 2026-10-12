"""Round-3 review records for the regrouped north hall and its new stair asset.

  python3 level-editor/blender/lincoln/keep_north_finalize_r3.py
Geometry is unchanged from round 2; the regroup moved node 182 into its own asset.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
RECIPE = str(Path(__file__).resolve().parent / 'refine_keep_north.py')
GROUND = 'native z = 220 (Blender Z = 268.57), the north-bailey plateau top'
COMMON = ['Covered state only; no interior or state patch applies to these nodes.',
          'Dimensions are 1-3 px manual measurements from covered.png.']
USER = ('needs refinement — stair should not be part of building (round 2, batch-3); earlier: '
        '"stairs should be separate and they are missing texture" (batch-1)')

ASSETS = {
 'lincoln-north-hall': dict(
  members='building-183, building-184, building-185 (stair 182 moved out)',
  carried=['185 hall body from the plateau (220) to 361 with a crenellated front parapet: 12 traced merlons plus 3 inferred west of x 1850 behind the keep.',
           '183/184 gable roof slopes (eaves 362/370, ridge 412).'],
  limits=['The west end of the hall (x < 1850) is hidden behind the keep; its merlons are inferred at the measured pitch.',
          'The hidden west gable meets the keep round tower only at the native footprint; any small gap there is not visible from the source camera.',
          'The east gable is a plain wall face; the separate stair asset stands against it (coplanar contact).'],
  hidden='North roof slope and the north and west walls are inferred.',
  response='The stair is no longer part of the building: this asset now holds only the hall body and roof (183/184/185). The stair is its own asset, lincoln-north-hall-stair.'),
 'lincoln-north-hall-stair': dict(
  members='building-182',
  carried=['182: closed 18-step stone stair from the curtain wall walk (native 370) to the courtyard (220) along the hall east gable; step pitch measured from the art (~11.8 px per step).',
           'Receiver mask: the stair native silhouette mask247, with foreground tree masks 78/85 excluded.'],
  limits=['Upper treads are behind foreground foliage in the art and stay neutral; only the lower six treads carry source texture.',
          'Step count is derived from the visible tread spacing; the upper steps are hidden by foliage.',
          'The stair west face is coplanar with the hall east gable; a placed standalone stair has a plain side wall there.'],
  hidden='The upper treads, the underside and the west side face are inferred.',
  response='The stair is now a standalone asset (node 182 only), separate from the hall building, so it can be placed against any wall walk. Its lower treads are textured from its native mask247; the upper ones are behind foliage in the art.'),
}


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def run(asset):
    i = ASSETS[asset]
    ws = R / 'round-3/assets' / asset
    model, views = sha(ws / 'model.blend'), sha(ws / 'modified/views.json')
    audit = json.loads((ws / 'inspection/source-audit.json').read_text())
    if audit['model_sha256'] != model or audit['modified_views_sha256'] != views:
        raise ValueError(asset + ': stale audit')
    r2 = R / 'round-2/assets/lincoln-north-hall'
    reason = (f'Geometry is the round-2 refinement from {r2} (model sha256 {sha(r2 / "model.blend")}), regrouped by catalog v3 '
              f'without mesh changes. Members: {i["members"]}. The integrated-context review found no seam, gap or '
              'ownership defect exposed by the regroup.')
    limits = i['limits'] + COMMON
    ev = {str(ws / r): sha(ws / r) for r in ['inspection/source-audit.png', 'inspection/source-audit.json',
          'inspection/neighbour-context.png', 'modified/solid.png', 'modified/textured.png', 'modified/context.png']}
    cov = {'version': 1, 'asset_id': asset, 'status': 'PASS', 'model_sha256': model, 'modified_views_sha256': views,
           'inspected_views': list(range(8)),
           'method': 'Round 3: source-camera BVH ray cast of the saved regrouped model over the full context crop; audit '
                     'domain = pixels whose first hit is an owned mesh, classified by the working v3 mask rule and the '
                     'saved owned-material texel. All eight views, the context and neighbour renders were inspected.',
           'counts': audit['counts'],
           'observation': 'Membership matches the user request (' + i['members'] + '). ' + i['response'],
           'evidence': ev, 'limitations': limits}
    (ws / 'source-coverage-audit.json').write_text(json.dumps(cov, indent=1) + '\n')
    review = [f'# {asset} review (keep_north lane, round 3)', '', 'Geometry unchanged. ' + reason, '',
              f'Ground height: {GROUND}.', '', f'User feedback: "{USER}". {i["response"]}', '',
              '## Geometry carried forward', ''] + [f'- {c}' for c in i['carried']] + ['',
              '## Checks', '', f'- Source audit counts: {json.dumps(audit["counts"])}.',
              '- All eight views, context and inspection/neighbour-context.png inspected.', '',
              '## Inferred hidden geometry', '', i['hidden'], '', '## Remaining limitations', ''] + \
             [f'- {l}' for l in limits] + ['']
    (ws / 'review.md').write_text('\n'.join(review))
    cand = {'version': 1, 'asset_id': asset, 'status': 'ready-for-user', 'geometry_refined': False,
            'geometry_reviewed': True, 'inspected_views': list(range(8)), 'recipe': RECIPE,
            'model_sha256': model, 'modified_views_sha256': views, 'no_change_reason': reason,
            'changes': [], 'limitations': limits, 'geometry_approval': 'pending',
            'texture_generation': 'not-started', 'ground_native_z': 220,
            'source_comparison': 'inspection/source-audit.png',
            'user_feedback': {'text': USER, 'response': i['response']}}
    (ws / 'candidate.json').write_text(json.dumps(cand, indent=1) + '\n')
    print(asset, 'ok')


if __name__ == '__main__':
    for a in ASSETS:
        run(a)
