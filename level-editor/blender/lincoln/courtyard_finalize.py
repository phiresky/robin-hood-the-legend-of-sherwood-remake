"""Write the courtyard lane's hash-bound review records (pure Python).

For each workspace: source-coverage-audit.json (from the Blender-attributed
measurement in inspection/source-coverage.json), review.md and candidate.json.
Run after `refine_courtyard.py --packet --closeup --audit`; the records bind the
current model.blend and modified/views.json hashes.

Usage: python3 courtyard_finalize.py <asset-id> [...]   (or --all)
"""
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
LINCOLN = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
RECIPE = HERE / 'refine_courtyard.py'

GROUND = ('Ground contact: native z 220 (Blender Z 268.57), the top of the inner/north bailey plateau '
          'volume under every footprint; all datum geometry below it removed.')
STATES = ('States inspected: covered.png and revealed.png are pixel-identical over this asset and no '
          'base/mission patch (Patch01-09, Pont_levis, patch08, mecanisme) overlaps it, so the covered '
          'exterior packet is the only state.')
STALE = ('Neighbour geometry inside this frozen workspace is the untouched baseline (datum pillars '
         'extruded from z 0); it occludes some of this asset\'s own artwork from the source camera. '
         'Those pixels are neutral in this packet and must be reprojected once the neighbouring lanes '
         'are integrated.')

NOTES = {
    'lincoln-courtyard-shed': {
        'changes': [
            'Rebuilt every part from the plateau (220) instead of native z 0; the tall datum pillars are gone.',
            'Roof 188: 5-unit mono-pitch slab on the native top surface (358 at the rock, 303 at the eave); the '
            'native wedge underside is removed.',
            'Back wall 195 and west end wall 194 stand on the plateau and stop under the roof slab.',
            '193 rebuilt as the east verge (barge) board: a 12-unit board under the roof\'s east edge following '
            'the slope 358->302 (it was a full-height wall).',
            'Open front: timber header (with the drawn small windows) hung from the eave down to the lintel '
            '(native 276), knee walls 191/192 to the lintel, and three eave posts (pixel x 1812, 1897, 1962) '
            'added to the roof node.',
            'Ladder 224 is a 4-unit leaning plank from the plateau to the eave; privy 196 a mono-pitch '
            'body+roof on its native footprint; barrel 220 a 17-unit cylinder.',
            '072 (reject-all, native top exactly 220) kept only as a 2-unit concealed strip just below the '
            'plateau surface.',
            'Working masks: 193 now receives the barn masks 172/175; foreground masks 68 (bush), 121 '
            '(fence), 173 (hay), 174 (goat), 176 (manger) excluded from every barn receiver.',
        ],
        'inferred': [
            'Back wall and end walls behind the roof are hidden in the artwork; they close the shell at the '
            'native footprint.',
            'Interior floor, stall partition, goat, hay and manger are not modelled (their pixels stay with the '
            'ground or are excluded).',
        ],
        'limitations': [
            '072 has no source-visible surface; it is a sub-surface strip recording the native collision '
            'footprint (shows as a thin bar in isolated review renders).',
            'Interior stall artwork inside the open front has no receiver in this asset.',
        ],
    },
    'lincoln-bailey-thatched-cottage': {
        'changes': [
            'Rebuilt as a gabled thatched cottage on the plateau: walls on the native eave footprint, 8-unit '
            'thatch slabs extended 7 units past the eaves (the drawn eave is ~9 px below the obstacle eave).',
            'West gable extended 8 units to the drawn west wall/verge (pixel x ~1300) with a 5-unit verge.',
            'East verge moved ~5 px west to the drawn verge; the east gable wall is set 21 px under the thatch '
            'overhang, matching the drawn front wall end at pixel x ~1483.',
            'Front slope split between 349 and 351 along a gable-parallel line; ridge unified at native 335.',
            'Chimney 363 starts inside the roof; stile 355 and barrel 356 stand on the plateau.',
            '364 (reject-all volume spanning both cottages) reduced to a concealed core wholly inside this '
            'cottage\'s walls, so it no longer penetrates or occludes the shingle cottage.',
            'Working masks: bush 67 and low fence 123 excluded from the thatch receivers.',
        ],
        'inferred': ['Rear (north) slope and wall are hidden; they mirror the drawn front construction.'],
        'limitations': [
            'The shingle cottage workspace still contains the baseline 364 box as context and must be reprojected '
            'after this asset is integrated.',
        ],
    },
    'lincoln-bailey-shingle-cottage': {
        'changes': [
            'Rebuilt as a gabled shingle cottage on the plateau: walls on the native eave footprint, 4-unit roof '
            'slabs with 5-unit eaves and 4-unit verges; the east verge 354 continues the front slope.',
            'Chimney 362 starts inside the roof.',
            'Fence 357 extended from the picket gate along the drawn rail fence: posts at base pixels '
            '(1727,1458), (1785,1440), (1832,1405) and two rails per bay at the drawn rows (reviewed mask 120 '
            'already draws the whole fence).',
        ],
        'inferred': ['Rear slope and north wall hidden; west gable mostly behind the thatched cottage.'],
        'limitations': [
            'In this frozen workspace the thatched cottage context is the baseline: its 0..317 box 364 and '
            'datum roof pillars occlude a band across the shingle roof from the source camera. The band is '
            'neutral in this packet; the refined thatched asset (364 reduced to an interior core) removes the '
            'occluder, so reproject after integration.',
        ],
    },
    'lincoln-bailey-trough': {
        'changes': ['Trough 358 now stands on the plateau (220..229) instead of a 0..229 pillar.'],
        'inferred': [], 'limitations': [],
    },
    'lincoln-bailey-hay-cart': {
        'changes': [
            'Bed 359 kept at its authored tilt; added the drawn spoked wheel (hub at pixel 1858,1381, radius '
            '17 native, touching the plateau), the second wheel 62 units across and 10 units east (drawn under the bed east corner) and the shaft '
            'resting on the plateau (pixel 1806,1361).',
            'Rails 360/361 kept as 12-unit side boards; the drawn hay-rack spindles added as ten 2-unit posts '
            'measured from the mask column tops (tips at pixel rows 1308-1341).',
            'Working masks: fence 121 in front of the cart excluded.',
        ],
        'inferred': ['Second wheel and axle hidden behind the bed.'],
        'limitations': ['Spindle gaps show ground, so the rack is open rather than a solid panel.'],
    },
    'lincoln-bailey-well': {
        'changes': [
            'Shaft 373 rebuilt as a 44-unit round stone shaft on the plateau (220..236), 5 units back to match '
            'the drawn rim (pixel rows 1574..1614); the native diamond was its bounding box.',
            'Roof 374 kept as a thin tilted plank roof and carried on two posts from the rim (pixel x 1724, '
            '1761).',
        ],
        'inferred': ['Rear half of the shaft and the roof underside are hidden.'],
        'limitations': [],
    },
    'lincoln-south-wall-cottage': {
        'changes': [
            'Rebuilt as a N-S gabled cottage on the plateau: walls on the native eave footprint (hidden behind '
            'the south curtain), 4-unit roof slabs with 5-unit eaves and 4-unit verges; chimney 367 starts '
            'inside the roof.',
        ],
        'inferred': ['All walls are hidden behind the south curtain wall; the south gable meets the curtain.'],
        'limitations': [STALE + ' Here: south curtain central run 110/086.',
                        'The south gable reaches the curtain footprint (y ~1978+); the final contact depends on '
                        'the south-gate lane\'s wall thickness.'],
    },
    'lincoln-south-wall-lean-to': {
        'changes': [
            'Rebuilt as a mono-pitch shed on the plateau rising south to the curtain (307 -> 339), 4-unit roof '
            'slab with a 5-unit north eave.',
            'Working masks: shrub 153 over the west roof edge excluded (curtain 231 exclusion kept).',
        ],
        'inferred': ['Walls hidden behind the south-east curtain.'],
        'limitations': [STALE + ' Here: south-east curtain 099/084.'],
    },
    'lincoln-south-wall-thatched-store': {
        'changes': [
            'Rebuilt as a N-S gabled thatched store on the plateau: 7-unit thatch slabs with 6-unit eaves; '
            'chimney 371 starts inside the roof.',
        ],
        'inferred': ['Walls hidden behind the south-east curtain.'],
        'limitations': [STALE + ' Here: south-east curtain 099/084.'],
    },
    'lincoln-southeast-lean-to': {
        'changes': [
            'Rebuilt as a mono-pitch shed on the plateau rising east to the curtain (282 -> 308); 4-unit roof '
            'slab with 5-unit west eave; the drawn west wall and doorway are its west face.',
        ],
        'inferred': ['East side hidden behind the east curtain walk.'],
        'limitations': [STALE + ' Here: south-east corner turret 083.'],
    },
    'lincoln-archery-target-west': {'changes': [], 'inferred': [], 'limitations': []},
    'lincoln-archery-target-west-middle': {'changes': [], 'inferred': [], 'limitations': []},
    'lincoln-archery-target-east-middle': {'changes': [], 'inferred': [], 'limitations': []},
    'lincoln-archery-target-east': {'changes': [], 'inferred': [], 'limitations': []},
    'lincoln-northeast-yard-hutches': {
        'changes': [
            'Both hutches stand on the plateau: body 220..253 with a gabled lid to 264; plan depth shortened to '
            '45% about the back corner because the drawn feet sit ~15 px above the obstacle\'s front corner.',
            'Working masks: tree foliage 157/161 in front of the west hutch excluded.',
        ],
        'inferred': ['Legs are merged into the body; rear faces hidden.'],
        'limitations': [],
    },
    'lincoln-northeast-yard-props': {
        'changes': [
            'Barrel 178 rebuilt as a 19-unit lying cylinder on the plateau along its native footprint axis.',
            '179 (reject-all, no unambiguous mask) kept as a small block at its authored plan standing on the '
            'plateau (220..233).',
        ],
        'inferred': [],
        'limitations': ['179 stays neutral: the pale block drawn ~8 px west of it is not tied to it by any '
                        'reviewed mask.'],
    },
}
for _asset in ('west', 'west-middle', 'east-middle', 'east'):
    NOTES[f'lincoln-archery-target-{_asset}'] = {
        'changes': ['Straw butt rebuilt as an ellipsoidal bale on the plateau, split between its two nodes at '
                    'the native ridge, 5 units taller than the obstacle ridge and 3 units wider on the west '
                    'where the target disc is drawn' + ('; plan depth extended 4 units south to the drawn '
                    'straw base.' if _asset in ('east-middle', 'east') else '.')],
        'inferred': ['Back (north) of the bale hidden.'],
        'limitations': ['The disc is painted on the bale surface; no separate disc geometry.'],
    }


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def finalize(asset_id, round_name='round-1', notes=None, no_change_reason=None, findings=None,
             auto_stale=True, finalizer=None):
    workspace = LINCOLN / round_name / 'assets' / asset_id
    notes = notes or NOTES[asset_id]
    finalizer = finalizer or HERE / 'courtyard_finalize.py'
    measured = json.loads((workspace / 'inspection/source-coverage.json').read_text())
    recipe_report = json.loads((workspace / 'geometry-recipe.json').read_text())
    c = measured['counts']
    model_sha = sha(workspace / 'model.blend')
    views_sha = sha(workspace / 'modified/views.json')
    rejected = c['own_mask_in_domain_not_accepted']
    own_surface = c['own_mask_rejected_on_own_surface']
    foreign = {k: v for k, v in c['own_mask_rejected_first_hit'].items() if not k.startswith('own: ') and k != 'no hit'}
    accepted = c['accepted_view_pixels']
    checks = {
        'accepted outside own masks <= max(2 px, 0.1%) (nearest-sample edge)': c['accepted_outside_own_mask'] <= max(2, 0.001 * accepted),
        'accepted RGB preserved (>= 98% exact at nearest sample)': (c['accepted_rgb_exact_fraction'] or 0) >= 0.98,
        'own-surface rejections <= 5% of accepted': own_surface <= 0.05 * max(accepted, 1),
        'own artwork without geometry beyond 1 px <= 3% of own mask': c['own_mask_source_pixels_uncovered_beyond_1px'] <= 0.03 * max(c['own_mask_source_pixels'], 1),
    }
    status = 'PASS' if all(checks.values()) else 'FAIL'
    observation = (
        f"View 0 is the artwork's own camera. Domain {c['domain_view_pixels']} view px; accepted {accepted} "
        f"(RGB exact {c['accepted_rgb_exact_fraction']}); own artwork rejected {rejected} px, of which "
        f"{rejected - own_surface} are occluded by neighbour geometry ({', '.join(f'{k}: {v}' for k, v in foreign.items()) or 'none'}) "
        f"and {own_surface} lie on this asset's own surface (texel-edge sampling, self-occlusion at eaves/"
        f"edges, reject-all nodes). Neutral pixels outside own masks: {c['neutral_outside_own_mask']} "
        f"({', '.join(f'{k}: {v}' for k, v in c['neutral_owner_breakdown'].items())}). Own artwork without "
        f"any geometry: {c['own_mask_source_pixels_uncovered']} of {c['own_mask_source_pixels']} source px, "
        f"{c['own_mask_source_pixels_uncovered_beyond_1px']} farther than 1 px from the silhouette "
        f"(hand-traced mask edges and thin drawn details).")
    evidence_files = [workspace / 'inspection/source-coverage.png', workspace / 'inspection/source-coverage.json',
                      workspace / 'modified/views/view-0-textured.png', workspace / 'modified/views/view-0-known.png',
                      workspace / 'modified/views/view-0-solid.png']
    if (workspace / 'inspection/mask-revisions.png').exists():
        evidence_files.append(workspace / 'inspection/mask-revisions.png')
    audit = {
        'version': 1, 'asset_id': asset_id, 'status': status,
        'model_sha256': model_sha, 'modified_views_sha256': views_sha,
        'inspected_views': list(range(8)),
        'method': ('Audit domain = every pixel of the saved model in modified view 0 (azimuth 0, 35 deg '
                   'orthographic = source camera), mapped back to covered.png independently of the acceptance '
                   'masks. Own artwork = reviewed native masks minus reviewed foreground exclusions from the '
                   'working source-masks.json. Each own pixel rejected by the packet was attributed with a scene '
                   'ray cast from the source camera to the first object hit; neutral pixels outside own masks '
                   'were attributed to the native mask owning them; own artwork pixels were mapped forward to '
                   'test for missing geometry; accepted pixels were compared with the artwork RGB. All eight '
                   'modified views and the auto-framed close-up sheets were inspected visually.'),
        'checks': checks,
        'observation': observation,
        'measurements': measured,
        'evidence': {str(p): sha(p) for p in evidence_files},
        'limitations': notes['limitations'] + ([STALE] if auto_stale and foreign and not any('stale' in x or 'baseline' in x for x in notes['limitations']) else []),
    }
    (workspace / 'source-coverage-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
    nodes = recipe_report['nodes']
    node_lines = '\n'.join(f"- `{n}`: " + (f"{len(v['shells'])} closed shell(s), " if 'shells' in v else 'kept, ')
                           + f"{v['faces']} faces, native z {v['native_z_range'][0]}..{v['native_z_range'][1]}"
                           for n, v in sorted(nodes.items()))
    round_note = ''
    if no_change_reason:
        round_note += '\n## Round-2 decision: geometry unchanged\n' + no_change_reason + '\n'
    if findings:
        round_note += '\n## Round-2 findings (integrated neighbours, refitted cameras)\n' + '\n'.join('- ' + x for x in findings) + '\n'
    review = f"""# {asset_id}: geometry review (courtyard lane)

Recipe: `{RECIPE}` (shapes in `courtyard_shapes.py`, working-mask revisions in `courtyard_masks.py`,
audit in `courtyard_audit.py`). Re-run:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 --python {RECIPE} -- \\
        --workspace {workspace} --packet --closeup --audit
    python3 {finalizer} {asset_id}
{round_note}
## Ground height
{GROUND}

## Changes
{chr(10).join('- ' + x for x in notes['changes'])}

## Components
{node_lines}

Every shell was checked closed and manifold with no degenerate faces and positive volume before saving;
outside geometry hashes are unchanged ({recipe_report['outside_objects_preserved']} objects).

## Inferred hidden geometry
{chr(10).join('- ' + x for x in notes['inferred']) or '- None beyond closing each shell.'}

## Checks
- Silhouette fitted against the native masks over enlarged covered.png crops with pixel grids
  (`scratch/courtyard/fit-{asset_id}.png`), then against the saved model.
- All eight modified solid/textured/known views and the context inspected, plus auto-framed close-ups in
  `inspection/closeup/`.
- Source coverage (`source-coverage-audit.json`, `inspection/source-coverage.png`): {status}.
  {observation}

## States
{STATES}

## Unresolved defects and limitations
{chr(10).join('- ' + x for x in audit['limitations']) or '- None known.'}
"""
    (workspace / 'review.md').write_text(review)
    candidate = {
        'version': 1, 'asset_id': asset_id,
        'status': 'ready-for-user' if status == 'PASS' else 'fix-needed',
        'geometry_refined': not no_change_reason, 'geometry_reviewed': True,
        'inspected_views': list(range(8)),
        'recipe': str(RECIPE),
        'model_sha256': model_sha, 'modified_views_sha256': views_sha,
        'changes': [] if no_change_reason else notes['changes'],
        'limitations': audit['limitations'],
        **({'no_change_reason': no_change_reason} if no_change_reason else {}),
        **({'round_2_findings': findings} if findings else {}),
        'ground_native_z': 220,
        'geometry_approval': 'pending', 'texture_generation': 'not-started',
        'source_comparison': 'inspection/source-coverage.png',
    }
    (workspace / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    return {'asset_id': asset_id, 'audit': status, 'checks': checks}


if __name__ == '__main__':
    ids = sys.argv[1:]
    if ids == ['--all']:
        ids = json.loads((LINCOLN / 'worker-assignments.json').read_text())['assignments']['courtyard']
    for asset in ids:
        print(json.dumps(finalize(asset)))
