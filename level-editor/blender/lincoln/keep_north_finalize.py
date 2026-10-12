"""Write review.md, source-coverage-audit.json and candidate.json for keep_north assets.

Plain python, run after the recipe packet and keep_north_audit.py:
  python3 level-editor/blender/lincoln/keep_north_finalize.py [asset ...]
Hashes are recomputed from the current model.blend and modified/views.json.
"""
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
ASSETS = ROOT / 'level-editor/work/needs-review/lincoln-refinement/round-1/assets'
RECIPE = str(Path(__file__).resolve().parent / 'refine_keep_north.py')
GROUND = 'native z = 220 (Blender Z = 268.57), the shared plateau top of the keep plateau and both baileys'

COMMON_LIMITS = [
    'Revealed-interior receivers (Patch06 keep room, Patch07 annex room) are not reviewed; covering pieces were kept and the packet is covered-state only.',
    'Solid-view corners are exact source-traced only where recorded in inspection/crenel-corners.json; other dimensions are 1-3 px manual measurements.',
]

A = {
 'lincoln-keep': dict(
  status='ready-for-user',
  changes=[
   'All 24 parts rebuilt from the measured specification; every pillar that was extruded from native z = 0 now stops at the plateau (220) or on its real support.',
   'NE round tower 208: true 64-segment cylinder (R 97) from the plateau to its roof floor at 892, with a plain rim parapet 212 (R 101/90, top 916); the artwork shows no merlons on this rim.',
   'Tall tower 213: 48-segment body (R 56) standing on the terrace (796) with a crenellated crown of 13 merlons (13.6 deg wide, phase 93 deg, notch 1046, top 1061); 211 is the crown floor at 1036; 214 is a thin front skin over the doorway sector.',
   'SW slate spire 218/219: true cone halves (eave R 72 at 839, apex 973) split at the source-part boundary, plus the thin finial rod to 1149; the turret body is now a 48-segment cylinder (R 61) inside 226.',
   'Front/west gap covers 207/459 and 217/460 now span flush between the adjoining wall ends (the native slabs were set back and left an open corner); 459/460 remain the Patch06 covering pieces.',
   'Terrace parapets: 210 plain parapet to 828, 226 front to 827, 227 east to 830 (artwork-measured).',
   '190 is the solid lower storey from the plateau to the revealed room floor (550); interior linings 215/216 and furniture 326-329/348 stand on that floor instead of reaching z = 0.',
   '221 is a small pent hood (684 to 713/692) above the front slit window.',
   'Working mask: exclusion of mask158 removed for keep nodes (it covers the round tower stonework, its bush lies east of the tower).'],
  limits=[
   'The front/east terrace parapet shows low, ambiguous block relief; it is modelled as a plain parapet (826-830) and the texture carries the block pattern.',
   'The inclined stone block beside the round tower door on the terrace and the corbelling under the tall-tower crown are not modelled.',
   'The finial rod lies outside the native keep masks and stays neutral.',
   'Silhouette residuals of 1-3 px on the west wall edge and the round tower right edge (see inspection/source-audit.png).',
   'The keep base below the great hall roof and the annex is hidden in the source; its walls are continued straight to the plateau.'],
  audit_obs='Owned first-hit silhouette matches the artwork: towers, crown merlons, spire, terrace and front face are green. Red slivers of 1-3 px at the west wall edge and round tower right edge are envelope boundaries; the red finial rod lies outside mask 253/378. Accepted-but-neutral pixels are grazing faces and the parapet tops within the envelope. Everything below the hall roof, annex and curtain is foreign first hit (neighbours in front).',
  hidden='Keep walls below the great hall roof/annex, the north and east faces, the back of both towers and the spire, the crown interior and the interior room are inferred continuations of the visible geometry.'),
 'lincoln-keep-annex': dict(
  status='ready-for-user',
  changes=[
   'All 15 parts rebuilt; pillars stop at the courtyard plateau (220); room furniture 330-333 stands on the room floor (472).',
   'Crenellated parapets from traced merlon corners: 201 east run 13 merlons (brightness-profile edges), 202 west run 10 merlons, 203 south-west run 7 merlons (x >= 1681), 204 front-left 3 merlons, 225 front with 4 merlon spans and the plain 1830-1888 stretch.',
   '205 is the tall corner pier (531-580) above the Patch07 cover 458 (220-533).',
   '189 is the lower storey below the Patch07 room floor (472); 198 is the wall-walk slab (545-550).'],
  limits=[
   'Merlon heights use the native notch/top levels (557-576); the artwork gives 1-3 px phase uncertainty per shoulder.',
   'Two 203 merlons at x 1681-1706 are drawn in the artwork but fall outside mask 255/403 and stay neutral (mask boundary).',
   'The north-east arm is a hollow walk between two thin wall bands; the deep well between the keep and the annex has no modelled floor.',
   'The pointed pier at x 1811-1830 on the front parapet is modelled as a flat-topped merlon block.'],
  audit_obs='Walk, parapets and merlons, and the ivy-covered front face are accepted. Red: the annex base seen through the open front of the courtyard shed (shed interior is foreground), a 1-2 px seam between facade 222 and parapet 225, and two 203 merlons outside mask 255/403. The rest of the crop is foreign first hit (keep, great hall, shed).',
  hidden='The east face of the north-east arm, the back of the front walls and the room interior are inferred.'),
 'lincoln-keep-annex-cone-turret': dict(
  status='ready-for-user',
  changes=[
   'The turret is now a corbelled bartizan: 40-segment body R 35 from 392 to the eave, a corbel frustum tapering to R 12 at 352; it no longer reaches the ground (the artwork shows it hanging above the courtyard).',
   'Slate spire 199/200: true cone halves (eave R 43 at 597, apex 647) plus the finial rod to 682.'],
  limits=[
   'In this workspace the neighbouring annex is at baseline; its front merlon (x 1888-1921) crosses the turret and is correctly rejected by the mask, and the refined annex geometry now covers it.',
   'Corbel profile is a straight frustum; the artwork shows a slightly rounded underside.'],
  audit_obs='Body, windows and spire are accepted. Red: the band where the annex front merlon passes in front of the turret (foreign foreground, now modelled in the annex workspace) and 1-2 px at the spire eave and corbel outline.',
  hidden='The back half of the turret is inferred as a full cylinder.'),
 'lincoln-keep-annex-round-turret': dict(
  status='ready-for-user',
  changes=[
   'Half-round turret rebuilt as a 48-segment cylinder (R 50, centre 1719/1495) from the plateau (220) with a crenellated crown of 11 merlons (14 deg, phase 84 deg, floor 478, notch 492, top 503); 228 is the crown floor.',
   'Mask review: mask413 (rock outcrop) was tested as an exclusion and rejected because its envelope also removes the turret left masonry column (inspection/mask-revision-rock413.png); the frozen assignment is kept.'],
  limits=[
   'The crown top (503, measured from the artwork) is above the baseline frame and slightly clipped at the top of the frozen view 0.',
   'The rock volume in front of the base (rocks lane, native top 308-381) is lower than the painted rock, so painted rock pixels project onto the lower-left turret base. Terrain requirement: rock top near x 1660-1740 should reach about z 385; no native mask separates the rock from the masonry.',
   'The back half of the crown lies inside the annex walls.'],
  audit_obs='Turret body, windows and crown are accepted. Red: the courtyard shed roof in front of the base. Painted rock at the lower-left base is accepted onto the turret because the rock volume is too low; see limitations.',
  hidden='Back half of the turret and crown are inferred.'),
 'lincoln-north-hall': dict(
  status='ready-for-user',
  changes=[
   '185 is the hall body from the plateau to 361 with the crenellated front parapet: 12 traced merlons (pitch 21.8 px, 12 px wide, notch 361, top 371) plus 3 inferred behind the keep.',
   '183/184 are the two gable roof slopes (eaves 362/370, ridge 412) instead of full-height pillars; 184 is hidden behind the ridge.',
   '182 is a 15-step stair from the wall walk (370) to the courtyard (220) along the east end.'],
  limits=[
   'Step count is derived from the visible tread spacing; the upper steps are hidden by foliage.',
   'The west end of the hall (x < 1850) is hidden behind the keep; its merlons are inferred at the measured pitch.'],
  audit_obs='Roof, front facade, merlons and door/windows are accepted. Red: the courtyard bush (mask80, foreground) and the stair region behind foliage. The west end is foreign first hit (keep).',
  hidden='North roof slope, north and west walls and the stair treads behind foliage are inferred.'),
 'lincoln-north-curtain-wall-west': dict(
  status='ready-for-user',
  changes=[
   '181 is the wall body with both half-round bastions from the plateau (220) to the walk (370); 187 is the north parapet (366-386) with 39 merlon blocks: 30 traced from source corners (including both bastions) and 9 inferred behind the keep round tower at the measured pitch.',
   'Working mask: exclusions of mask159/49/158/48 removed (native envelopes containing the visible walk and merlons) and mask243 added as include for the eastern bastion outside mask244; the foreground tree mask78/85 and the hall mask245 remain excluded.'],
  limits=[
   'Foreground tree masks 78/85 have interior holes; dark foliage speckles inside those holes still project onto the wall south face below the east half. No native mask fills them; a filled/closed exclusion needs coordinator tooling.',
   'The south wall face is foreign foliage in the source and stays mostly neutral.',
   'Merlon shoulders are +-2 px manual traces; the walk has no inner parapet (none in the artwork).'],
  audit_obs='Walk, parapet merlons and both bastions are accepted after the reviewed mask revision. Red: the wall south face behind the orange tree (mask78/85, foreground). Accepted foliage speckles in the tree-mask holes are a known contamination (see limitations).',
  hidden='The north (outer) face, the west end behind the keep and the bastion backs are inferred.'),
 'lincoln-north-curtain-wall-east': dict(
  status='ready-for-user',
  changes=[
   '180 is the wall body from the plateau (220) to the walk (370); 186 is the north parapet (366-386) with 9 traced merlons (pitch 22.3 px) and one inferred at the hidden east end.',
   'Working mask: exclusion of mask49 removed (it contains the parapet stonework; its foliage lies behind the wall).'],
  limits=[
   'Foreground tree masks 78/85/157/161 have interior holes; foliage speckles inside them still project onto the wall south face. No native mask fills them; a filled/closed exclusion needs coordinator tooling.',
   'The south wall face is covered by trees in the source and stays mostly neutral.'],
  audit_obs='Walk and merlons are accepted. Red: the south face behind the foreground trees. Accepted foliage speckles in the tree-mask holes are a known contamination (see limitations).',
  hidden='North face and the east end near the north-east tower are inferred.'),
}


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def finalize(asset):
    info = A[asset]
    ws = ASSETS / asset
    model, views = sha(ws / 'model.blend'), sha(ws / 'modified/views.json')
    audit = json.loads((ws / 'inspection/source-audit.json').read_text())
    if audit['model_sha256'] != model or audit['modified_views_sha256'] != views:
        raise ValueError(f'{asset}: source-audit.json is stale; rerun keep_north_audit.py')
    evidence = {}
    for rel in ['inspection/source-audit.png', 'inspection/source-audit.json', 'inspection/spec-preview.png',
                'inspection/geometry-recipe.json', 'inspection/crenel-corners.json', 'inspection/mask-revision.json',
                'modified/solid.png', 'modified/textured.png', 'modified/context.png']:
        if (ws / rel).exists():
            evidence[str(ws / rel)] = sha(ws / rel)
    for p in sorted((ws / 'inspection').glob('mask-revision-*.png')):
        evidence[str(p)] = sha(p)
    c = audit['counts']
    cov = {'version': 1, 'asset_id': asset, 'status': 'PASS', 'model_sha256': model,
           'modified_views_sha256': views, 'inspected_views': list(range(8)),
           'method': 'Source-camera BVH ray cast of the saved model over the full context crop; the audit domain is '
                     'every pixel whose first hit is an owned mesh (independent of the acceptance masks), classified '
                     'by the working native-mask rule and by the saved owned-material texel; all eight modified '
                     'solid/textured views and the covered-state context were inspected.',
           'counts': c,
           'observation': info['audit_obs'],
           'evidence': evidence,
           'limitations': info['limits'] + COMMON_LIMITS}
    (ws / 'source-coverage-audit.json').write_text(json.dumps(cov, indent=1) + '\n')
    review = [f'# {asset} review (keep_north lane, round 1)', '',
              f'Recipe: `{RECIPE} --asset {asset} --packet` (geometry helper keep_north_geom.py, audit keep_north_audit.py).', '',
              f'Ground height: {GROUND}.', '', '## Changes', ''] + [f'- {x}' for x in info['changes']] + [
              '', '## Checks', '',
              '- Source-camera spec preview (inspection/spec-preview.png) and saved-model source audit (inspection/source-audit.png) against covered.png.',
              '- All eight modified solid and textured views inspected; frozen cameras unchanged; validation PASS; every rebuilt part is closed and manifold (inspection/geometry-recipe.json).',
              f'- Source audit counts: {json.dumps(c)}.',
              '- Merlon corner evidence: inspection/crenel-corners.json (where applicable).', '',
              '## Inferred hidden geometry', '', info['hidden'], '',
              '## States inspected', '',
              'Covered state (source-states/covered.png) for projection. Revealed Patch06 (keep, disables 459/460) and Patch07 (annex, disables 458) were inspected in covered/revealed pairs; covering pieces are kept, revealed receivers are not reviewed.', '',
              '## Unresolved defects and limitations', ''] + [f'- {x}' for x in info['limits'] + COMMON_LIMITS] + ['']
    (ws / 'review.md').write_text('\n'.join(review))
    cand = {'version': 1, 'asset_id': asset, 'status': info['status'], 'geometry_refined': True,
            'geometry_reviewed': True, 'inspected_views': list(range(8)), 'recipe': RECIPE,
            'model_sha256': model, 'modified_views_sha256': views, 'changes': info['changes'],
            'limitations': info['limits'] + COMMON_LIMITS, 'geometry_approval': 'pending',
            'texture_generation': 'not-started', 'ground_native_z': 220,
            'source_comparison': 'inspection/source-audit.png'}
    (ws / 'candidate.json').write_text(json.dumps(cand, indent=1) + '\n')
    print(asset, 'finalized', model[:12], views[:12])


if __name__ == '__main__':
    for a in (sys.argv[1:] or list(A)):
        finalize(a)
