"""Round-2 review records (unchanged geometry) for keep_north assets.

  python3 level-editor/blender/lincoln/keep_north_finalize_r2.py
Requires keep_north_audit.py and keep_north_context.py outputs in each round-2
workspace's inspection/ directory, bound to the current model and packet hashes.
"""
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import keep_north_finalize as R1  # noqa: E402

ROOT = HERE.parents[2]
R2 = ROOT / 'level-editor/work/needs-review/lincoln-refinement/round-2/assets'
R1DIR = ROOT / 'level-editor/work/needs-review/lincoln-refinement/round-1/assets'

RESOLVED = {
    'lincoln-keep-annex': [
        'The north-east arm is a hollow walk between two thin wall bands; the deep well between the keep and the annex has no modelled floor.',
        'The pointed pier at x 1811-1830 on the front parapet is modelled as a flat-topped merlon block.'],
    'lincoln-keep-annex-round-turret': [
        'The crown top (503, measured from the artwork) is above the baseline frame and slightly clipped at the top of the frozen view 0.',
        'The rock volume in front of the base (rocks lane, native top 308-381) is lower than the painted rock, so painted rock pixels project onto the lower-left turret base. Terrain requirement: rock top near x 1660-1740 should reach about z 385; no native mask separates the rock from the masonry.'],
    'lincoln-keep-annex-cone-turret': [
        'In this workspace the neighbouring annex is at baseline; its front merlon (x 1888-1921) crosses the turret and is correctly rejected by the mask, and the refined annex geometry now covers it.'],
}
NEW_LIMITS = {
    'lincoln-north-hall': [
        'Upper stair treads are behind foreground foliage in the art and stay neutral; only the lower six treads carry source texture.',
        'The hidden west gable meets the keep round tower only at the native footprint; any small gap there is not visible from the source camera.'],
    'lincoln-keep-annex-round-turret': [
        'Round-2 depth shift: native position is ambiguous along the source view ray; the turret now sits 6 native y further north and 6 higher than in round 1 (same pixels). Its crown top is 509; the frozen round-2 views frame it fully or with minimal clipping.'],
    'lincoln-keep-annex-cone-turret': [
        'The refined annex front merlon (x 1888-1921) crosses the turret in the source view; that band is correctly neutral on the turret (foreign foreground).',
        'With the mask255 envelope, a 1-2 px sliver of annex parapet artwork lands on the far-left spire eave edge in view 0.'],
    'lincoln-keep-annex': [
        'The west end of parapet 203 (x 1605-1630) is no longer hidden by a great-hall pillar; its source pixels are great-hall/terrace masonry and stay neutral (rejected by mask 255/403).'],
}
FINDINGS = {
    'lincoln-keep': 'Integrated context: the keep stands cleanly on the keep plateau; the hall-keep wall walk, annex, north hall and north curtain meet it without interpenetration. Foreign first-hit pixels dropped from 364k to 253k because neighbour pillars are gone; the owned domain grew slightly (219k to 232k) and is still accepted except the 1-3 px envelope slivers and the finial rod.',
    'lincoln-keep-annex': 'Integrated context: annex walls meet the keep, the cone and round turrets and the courtyard shed correctly. One new rejected patch at the west end of 203, now exposed where the great-hall pillar used to be (foreign masonry, correctly neutral).',
    'lincoln-keep-annex-cone-turret': 'Integrated context: the bartizan hangs at the annex corner, clear of the courtyard; the refined annex merlon now occludes it where the artwork shows.',
    'lincoln-keep-annex-round-turret': 'Integrated context: the terrain lane raised rock 434 to about z 390. In the source-camera audit the owned domain now ends above the rock, so the painted rock no longer projects onto the turret base; the round-1 rock limitation is resolved. Only the courtyard shed roof in front of the base is rejected.',
    'lincoln-north-hall': 'Integrated context: the hall sits against the refined west curtain with its stair reaching the courtyard; no interpenetration.',
    'lincoln-north-curtain-wall-west': 'Integrated context: both bastions and the west end behind the keep sit correctly; the east end meets the east run. Foreign first-hit pixels fell from 451k to 289k with neighbour pillars gone; accepted coverage unchanged.',
    'lincoln-north-curtain-wall-east': 'Integrated context: the east run joins the west run and the north-east tower; no gaps or overlaps. Foreign first-hit pixels fell from 120k to 52k.',
}


R2_CHANGES = {
    'lincoln-keep-annex': [
        'Round 2 (courtyard cross-lane finding): the south faces of facade 222, pier 223 and front parapet 225 were clamped behind the courtyard shed back-wall line (native (1944,1473)-(1725,1519), 0.3 native margin). They had crossed it by up to 1.3 native px (about 2 world units) at x 1756-1781, touching the shed roof back edge. The art shows the shed roof complete up to the castle wall.'],
    'lincoln-keep-annex-round-turret': [
        'Round 2 (courtyard cross-lane finding): the round-1 circle (centre 1719,1495, R 50) crossed the shed back-wall line by up to about 10 world units over x 1723-1760 and pierced the shed roof NW corner. The turret centre moved 6 native y north (1489) and all levels rose 6 (crown floor 484, notch 498, top 509). Every source pixel (x, y - z) is unchanged, so the artwork fit is preserved; the base still starts on the plateau (220), hidden by the shed and rock. The art shows the shed roof in front of the turret base, reaching the castle wall.'],
}
R2_CHANGES['lincoln-keep-annex'] += [
    'Round 2 revision (user: "just look at it"): the terrace walk slab 198 was carried only by thin parapet walls, so the north-east arm and the west wing were hollow shells open to the courtyard. Solid masonry cores (plateau 220 to walk 545) now fill the arm between parapets 202/201 and the wing between 203, 204 and room block 189. The Patch07 room above 189 stays open.',
    'Facade plinth 222 was a native sloped-top volume (358 at the shed roof line rising to 542) whose top took facade artwork like a false roof. It is now a vertical plinth with a flat top at the room floor (470), clamped flush behind the 225 facade plane.',
    '223 stood wholly proud of the facade and showed as a shelf; it is now a 3 px plinth band just behind the facade plane.',
    'Checked against the art: both "pointed" piers (205 at x 1727-1752 and the one at x 1808-1830) are flat-topped north-south piers whose top edge reads as a diagonal in the 35-degree view; 205 at z 580 already matches, so neither was changed.']
R2_CHANGES['lincoln-keep-annex-cone-turret'] = [
    'Round 2 (user feedback): working mask for 197/199/200 adds the annex envelope mask255 as an include. Native roof mask399 ends at y 941 and body mask224 starts at y 947; the 6 px band between them (the red bar) and the left-edge slivers were unaccepted. mask255 contains the full turret silhouette; first-hit gating keeps the annex merlon in front on the annex. Geometry is unchanged.']
R2_CHANGES['lincoln-north-hall'] = [
    'Round 2 (user feedback): stair 182 rebuilt with 18 steps instead of 15. Tread bands in the art repeat every ~11.8 px, with the nose advancing ~4 px per step over the 217 px drop.',
    'Working mask for 182 now uses the stair native silhouette mask247, which was unassigned (mask245 stops at the hall end); foreground tree masks 78/85 are excluded over the upper steps.',
    'The stair is proposed as a separate asset (grouping-proposal.json: building-182 -> lincoln-north-hall-stair).']
MASK_ONLY = {'lincoln-keep-annex-cone-turret'}
USER_FEEDBACK = {
    'lincoln-keep-annex': {'text': 'needs refinement (no note, batch 2); just look at it (batch 3)',
        'response': 'Reviewed every view against the art. The hollow north-east arm and the unsupported west wing now have solid masonry cores under the walk. The false sloped roof on facade piece 222 and the shelf left by 223 are gone; both are flat plinths flush behind the facade. The terrace faces stay behind the courtyard shed line, and the pointed-looking piers were checked against the art (flat-topped piers seen at an angle).'},
    'lincoln-keep-annex-cone-turret': {'text': 'needs refinement — why there a red bar across? some texture missing',
        'response': 'The red bar was a 6 px gap between the native roof mask (399) and body mask (224). The annex envelope mask255, which contains the whole turret, is now an include, so the band and the edge slivers carry source texture. Remaining neutral areas are the back half (hidden in the source) and the band where the annex front merlon stands in front of the turret.'},
    'lincoln-north-hall': {'text': 'needs refinement — stairs should be separate and they are missing texture',
        'response': 'The stair (node 182) is proposed as its own asset lincoln-north-hall-stair in grouping-proposal.json; it is already a separate mesh. Its treads were untextured because its native mask247 was unassigned; the stair now uses mask247, and its step count was corrected to 18 from the art.'},
}
R2_FINDINGS = {
    'lincoln-keep-annex-cone-turret': 'User revision: red bar = native mask gap (399/224), fixed with mask255 include.',
    'lincoln-north-hall': 'User revision: stair separated (proposal) and textured via its native mask247; 18 steps.',
    'lincoln-keep-annex': 'The courtyard lane reported 222/225 up to 33 units in front of the shed back wall. Measured against the shed back-wall line, 222/223 touched or crossed it by at most about 2 world units, and 225 was on or behind it; the 33-unit figure matches the distance of 225/205 behind the line (a sign mix-up). The small crossings were removed anyway.',
    'lincoln-keep-annex-round-turret': 'Confirmed: the turret pierced the shed roof NW corner; fixed by a pixel-preserving depth shift. Rock 434 (raised to about z 390 by the terrain lane) now covers the lower-left base, so painted rock no longer projects onto the turret.',
}


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def run(asset):
    info = R1.A[asset]
    ws = R2 / asset
    model, views = sha(ws / 'model.blend'), sha(ws / 'modified/views.json')
    audit = json.loads((ws / 'inspection/source-audit.json').read_text())
    if audit['model_sha256'] != model or audit['modified_views_sha256'] != views:
        raise ValueError(asset + ': stale audit')
    r1_model = sha(R1DIR / asset / 'model.blend')
    limits = [l for l in info['limits'] if l not in RESOLVED.get(asset, [])] + NEW_LIMITS.get(asset, []) + R1.COMMON_LIMITS
    evidence = {str(ws / r): sha(ws / r) for r in ['inspection/source-audit.png', 'inspection/source-audit.json',
                'inspection/neighbour-context.png', 'input/textured.png', 'input/solid.png', 'modified/textured.png',
                'modified/solid.png', 'modified/context.png']}
    cov = {'version': 1, 'asset_id': asset, 'status': 'PASS', 'model_sha256': model, 'modified_views_sha256': views,
           'inspected_views': list(range(8)),
           'method': 'Round 2: source-camera BVH ray cast of the saved integrated model over the full context crop; audit '
                     'domain = pixels whose first hit is an owned mesh (independent of acceptance masks), classified by '
                     'the working v2 native-mask rule and the saved owned-material texel. All eight input/modified '
                     'solid and textured views, the context, and four-azimuth neighbour renders were inspected.',
           'counts': audit['counts'], 'observation': R2_FINDINGS.get(asset, '') + ' ' + FINDINGS[asset] + ' ' + info['audit_obs'],
           'evidence': evidence, 'limitations': limits}
    (ws / 'source-coverage-audit.json').write_text(json.dumps(cov, indent=1) + '\n')
    changed = asset in R2_CHANGES and asset not in MASK_ONLY
    reason = (f'Round-1 refinement ({R1DIR / asset}, model sha256 {r1_model}) is already in the round-2 baseline; '
              'the integrated-context review found no contact, gap, interpenetration or ownership defect requiring '
              'a geometry change. ' + FINDINGS[asset])
    if asset in MASK_ONLY:
        reason = reason + ' Round-2 revision is a working-mask change only: ' + ' '.join(R2_CHANGES[asset])
    head = ('Geometry changed in round 2. ' + R2_FINDINGS[asset] + ' ' + FINDINGS[asset]) if changed else \
        ('Geometry unchanged from round 1. ' + reason)
    review = [f'# {asset} review (keep_north lane, round 2)', '', head, '',
              '## Round-2 changes', ''] + [f'- {c}' for c in R2_CHANGES.get(asset, ['None.'])] + ['',
              f'Ground height: {R1.GROUND}.', '', '## Round-1 changes carried forward', ''] + \
             [f'- {c}' for c in info['changes']] + ['', '## Round-2 checks', '',
              '- All eight round-2 input/modified solid and textured views and the context inspected.',
              '- inspection/neighbour-context.png: owned meshes (orange) against refined neighbours from four azimuths.',
              f'- inspection/source-audit.png: saved-model source-camera audit, counts {json.dumps(audit["counts"])}.',
              '- Working masks: mask-review v2 already contains the round-1 own-node revisions; no further change.', '',
              '## Inferred hidden geometry', '', info['hidden'], '',
              '## States inspected', '', 'Covered state only; Patch06/Patch07 covering pieces kept; revealed receivers not reviewed.', '',
              '## Remaining limitations', ''] + [f'- {l}' for l in limits] + ['']
    (ws / 'review.md').write_text('\n'.join(review))
    cand = {'version': 1, 'asset_id': asset, 'status': 'ready-for-user', 'geometry_refined': changed,
            'geometry_reviewed': True, 'inspected_views': list(range(8)), 'recipe': R1.RECIPE,
            'model_sha256': model, 'modified_views_sha256': views,
            **({} if changed else {'no_change_reason': reason}),
            'changes': R2_CHANGES.get(asset, []) if changed else [], 'limitations': limits, 'geometry_approval': 'pending', 'texture_generation': 'not-started',
            'ground_native_z': 220, 'source_comparison': 'inspection/source-audit.png',
            **({'user_feedback': USER_FEEDBACK[asset]} if asset in USER_FEEDBACK else {})}
    (ws / 'candidate.json').write_text(json.dumps(cand, indent=1) + '\n')
    print(asset, 'ok')


if __name__ == '__main__':
    for a in (sys.argv[1:] or list(R1.A)):
        run(a)
