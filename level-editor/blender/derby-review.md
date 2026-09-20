# Derby asset refinement review

Every logical asset receives an independent geometry and projection review.
Reviewed does not mean finished: record remaining defects and uncertain geometry.
Workers use separate Blender copies; the primary session integrates reviewed
recipes, reruns layered projection, and publishes the map and standalone assets.

For each asset check every component against source artwork, reference and two
oblique views in textured and solid modes; inspect silhouette, roof/wall joins,
openings, stairs, surface orientation, texture ownership, hidden-face fallback,
group/part identity, repeated execution, and exported geometry/UVs. Keep original
collision data separate from refined rendering geometry.

## Current integration state — 2026-09-20

All 30 logical assets have received an individual worker pass. **All initial
passes are now merged and published, including the 30 standalone assets. The
assets are not finished.** “Published”
means the geometry is included in the latest map export, not that all defects
are resolved. West Cottage's second geometry pass and generated texture bake are
now published; the generated hidden details remain inferred.

Latest map publication: `work/derby-refinement/west-cottage-final-publish/derby.scene.glb`
to `library/scenes/derby-volumes.scene.glb`, with the document fingerprint updated.
It contains 30 groups, 270 canonical parts, 303 meshes, and 154 modeled steps.
Browser acceptance passed: 30 named groups, 270 parts, 14 stair assemblies,
154 steps, 145 reported crenel notches, 30 interior meshes and seven reveal patches.
Real ray picking selected Great Keep as a group first, then its part 146.
All 30 standalone descriptors/models were published from the same scene.

Working integration checkpoint: `derby-refinement.blend`, with evidence in
`all-assets-integration/changes.json` and `all-assets-integration/reprojection/`.
The combined projection refreshed 40,864 faces and retained 127,202 fallback faces.
Well ground-ghost cleanup is now applied (1,935 pixels), and 21 mesh names synced.
West Cottage's second pass fixes nonplanar roof creases, eave thickness and
grazing-wall projection stretches. The initial `lower-west-cottage-sunburst`
input is superseded by `lower-west-cottage-shaded-v2`.

Paths below are relative to `level-editor/work/derby-refinement/`.
Reusable geometry recipes are in `level-editor/blender/derby_asset_*.py`.

| Asset | Integration | Latest work / remaining review |
| --- | --- | --- |
| South Gatehouse | Published **with texture seams; active rework** | User clarified source solid geometry looks correct. Exact roof triangle comparison exonerated export topology. Generated roof texture used unsupported UV channel 4; exporter now compacts material-used channels and the editor-only hole artifact is fixed. Round-roof rebuild paused. Generated/fallback seams remain; `south_gatehouse_asset` improving projection coverage in an isolated worker. |
| Southwest Postern Tower | Published | Timber ladders, scaffold, and 8 crenels; `postern-worker-v2`. |
| Lower Bailey East Curtain | Published | 25 crenels and closed supports; lower-east-curtain worker series. |
| Lower Bailey West Curtain | Published | 25 crenels, rounded turret, curved roof sectors, 18 stairs; oblique joins remain reviewable. |
| Lower Bailey Well (formerly courtyard prop) | Published | Hollow shaft, iron lifting frame, rope, pulley, bucket; `lower-well-worker-v2`. Printed ground ghost cleanup integrated. |
| Southern Approach Stone | Published | Closed angular outcrop, replacing tent-like wedges; `approach-stone-worker/worker-v2`. |
| Lower Bailey East Cottage | Published | Hipped roof, closed walls, barrel; `lower-east-cottage`. |
| Lower Bailey West Cottage | V2 geometry and generated texture published | Continuous hip boundary, corrected front eave, thick thatch/overhang, porch rails; eight closed components. Failed window recess experiment removed. Fresh source projection at cosine cutoff .25, no-mask Sunburst high generation, per-texel atlas bake. Geometry: `lower-west-cottage-v2-boundary/review.blend`; mapped eight-view result: `west-cottage-texture-baked-v2/eight-views.png`. Concealed windows/timber patterns are synthesized, not recovered facts. |
| Lower Bailey Southwest Cottage | Published | Thatched eave and damaged-roof recess; `southwest-cottage/reviewed`. |
| Lower Bailey Southeast Cottage | Published | Closed end wedge, porch recess, thick roof; `southeast-cottage-pass3`. |
| Lower Bailey Northwest Cottage | Published | Hipped roofs, barrel, fence, wash tub; `northwest-cottage-final-v5`. |
| East Bailey South Curtain Lean-to | Published | Chimney, thick roof and walls; `south-shelter-pass2`. |
| East Wall Lean-to Shelter (formerly wall landing) | Published | Closed body and thick shelter roof; `east-landing-worker-v2`. |
| East Bailey North Curtain Timber Shed | Published | Chimney, annex, main shed and doorway recess; `north-shelter-pass3`. |
| Lower Bailey Supply Cart (formerly stair supplies) | Published | Cart, two spoked wheels, barrel and hoops; `east-supplies/accepted`. |
| East Bailey Gatehouse | Published | Closed gate assembly, arch, roof, rounded turret, 7 crenels; `east-bailey-gate/final.blend`. |
| East Bailey East Curtain | Published | 31 crenels and closed support geometry; `east-bailey-east-curtain` recipe. |
| East Hall Exterior Stair | Published | Corrected painted/geometric mismatch to 13 treads; trough refined; `stair-texture-fix/final`. |
| East Bailey West Curtain | Published | 19 crenels; some dark fallback texture remains; `east-bailey-west-worker-v4`. |
| East Bailey Stacked Timber | Published | 51 closed logs with staggered stacking; `stacked-timber-worker` v6. |
| East Bailey Covered Well | Published | Timber canopy, hollow shaft, bucket; `east-well-final-v7`. Ground bucket ghost cleanup integrated. |
| East Bailey Barrel (formerly supply crate) | Published | Lying barrel with two hoops; verify authored hoop materials survive reprojection before integration. |
| East Bailey Chopping Block (formerly yard prop) | Published | Stump, axe head and handle; `east-bailey-yard-prop-worker2`. |
| Upper Bailey West Curtain | Published | 21 crenels, 13 treads, thick covered roof; `upper-west-worker-v3`. |
| Upper Bailey Gatehouse | Published | Open arch, closed components, 11 physical crenels; closure and interior parts retained; `upper-gatehouse-agent/worker-v2`. |
| Keep to East Hall Bridge | Published | Arch void, deck-height parapet, coping and closed landing; `bridge-worker/worker-v2`. |
| Upper Bailey East Curtain | Published | Closed walk and 14 crenels; `upper-east-curtain-worker1`. |
| Great Keep | Published | 34 total crenels, closed roofs without ground curtains, interior stair corrections. Interior texture/fallback quality still needs scrutiny. |
| East Hall | Published | 19 crenels and architecture corrections; `east-hall-complete-review-v3`. |
| East Watchtower | Published | 7 crenels, curved cap, trimmed shaft and supports; `east-watchtower/checked.blend`. Inherited part 215 still has 54 open edges; not declared manifold. |
| Ground, courtyard levels, slopes and foreground banks | Published | Terrain relief with supported footprints and source projection retained. Not a complete terrain-detail pass. |
| Distant landscape/background | Initial pass integrated | Inferred valley relief; distant geometry remains uncertain. |
| Ground prop ghost cleanup | Published | `derby_ground_cleanup.py`: 1,935 edited pixels, outside-mask identity and PNG round-trip checked; `ground-cleanup-worker/worker-v3.blend`. Requires refined wells before integration. |

## Projection and generated-texture status

- Exterior uses covered source artwork; 28 interior receiver parts use uncovered
  artwork in separate patch layers. Preserve reveal/sight metadata on export.
- Reprojection was rerun after the curved gatehouse roof pass. Further geometry
  changes require another projection pass before publication.
- Latest generation input: `multiview-shaded-curved/input.png`, with saved camera
  transforms in `views.json`. Original-camera tile uses source artwork clipped to
  the model silhouette; other views show unknown surfaces as shaded geometry.
- Selected experiment: `generation-short-no-mask/generated-raw.png` beneath that
  directory, generated with `gpt-image-2.5-sunburst`, `quality: high`.
- Published preview initially applied the composite to 82 rear-facing faces,
  choosing one camera per face and skipping 104 hidden faces. That approach caused
  obvious triangular mismatches and is superseded in the working scene.
- `apply_multiview_texture.py` now bakes per-texel visibility into a packed atlas.
  Reviewed candidate `sunburst-projection-baked-v4/worker.blend` is merged in the
  working scene and published. All 17 gate objects' geometry is unchanged.
  The atlas covers 238 faces; the original-view mean RGB drift is 0.432/255 due to
  resampling. Large wall triangles are resolved; oblique roof shading seams remain.
  Eight-view evidence: `sunburst-projection-baked-v4/after-sheet.png`.
- Pre-texture recovery checkpoint: `sunburst-editor-preview/before-texture.blend`.
- Controlled API mask tests selected the wrong square in both cases despite
  verified multipart mask bytes. No-mask generation is the current experiment.

## Next required work

1. Keep the reviewed gatehouse shape while resolving texture artifacts. Export UV
   compaction fixed the editor-only hole; source and exported roof triangles match.
   Do not reshape the roof to compensate for a texture bug. Internal nonplanar caps
   remain a separate potential cleanup, not a demonstrated cause of this defect.
2. Improve generated rear texture placement and verify the original-camera render
   remains unchanged. Do not infer texture correctness from the generated sheet.
3. West Cottage's second geometry pass, fresh projection, shaded input generation
   and no-mask texture generation are complete. Review remaining fine-detail and
   hidden-surface consistency defects in the baked eight-view render.
4. After subsequent changes, export/publish map and standalone assets together and
   repeat editor loading, selection, UV and reveal checks as appropriate. Initial
   combined publication and checks are complete.
5. Full-map comparisons for the combined geometry are in
   `all-assets-integration/full-map/reference-{solid,textured}.png`; refresh after
   the next cottage geometry change and keep this table current.

Source art cannot establish unseen geometry uniquely. Record remaining defects
instead of treating a completed worker pass as a perfect model.
