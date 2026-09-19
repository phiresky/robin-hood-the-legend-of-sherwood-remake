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

## Current integration state — 2026-09-19

All 30 logical assets have received an individual worker pass. **They are not all
merged or finished.** “Worker ready” below means a recipe and isolated evidence
exist; it does not mean the live editor contains that refinement. “Published”
means the geometry is included in the latest map export, not that all defects
are resolved. Standalone asset exports still need synchronization with that map.

Latest map publication: `work/derby-refinement/sunburst-editor-uv-fixed/derby.scene.glb`
to `library/scenes/derby-volumes.scene.glb`, with the document fingerprint updated.
It contains 30 groups, 270 canonical parts, 284 meshes, and 141 modeled steps.
The newest publication passed a focused real-browser gatehouse load/render check
(30 groups, 270 parts). It has not yet passed the complete browser acceptance run.
The earlier check's fixed staircase counts must be updated for current geometry.

Paths below are relative to `level-editor/work/derby-refinement/`.
Reusable geometry recipes are in `level-editor/blender/derby_asset_*.py`.

| Asset | Integration | Latest work / remaining review |
| --- | --- | --- |
| South Gatehouse | Published **with texture seams; active rework** | User clarified source solid geometry looks correct. Exact roof triangle comparison exonerated export topology. Generated roof texture used unsupported UV channel 4; exporter now compacts material-used channels and the editor-only hole artifact is fixed. Round-roof rebuild paused. Generated/fallback seams remain; `south_gatehouse_asset` improving projection coverage in an isolated worker. |
| Southwest Postern Tower | Published | Timber ladders, scaffold, and 8 crenels; `postern-worker-v2`. |
| Lower Bailey East Curtain | Published | 25 crenels and closed supports; lower-east-curtain worker series. |
| Lower Bailey West Curtain | Published | 25 crenels, rounded turret, curved roof sectors, 18 stairs; oblique joins remain reviewable. |
| Lower Bailey Well (formerly courtyard prop) | Published | Hollow shaft, iron lifting frame, rope, pulley, bucket; `lower-well-worker-v2`. Printed ground ghost cleanup ready separately, not integrated. |
| Southern Approach Stone | Worker ready | Closed angular outcrop, replacing tent-like wedges; `approach-stone-worker/worker-v2`. |
| Lower Bailey East Cottage | Published | Hipped roof, closed walls, barrel; `lower-east-cottage`. |
| Lower Bailey West Cottage | Published | Hipped roof and supporting walls; porch recess retained; `west-cottage-worker`. |
| Lower Bailey Southwest Cottage | Published | Thatched eave and damaged-roof recess; `southwest-cottage/reviewed`. |
| Lower Bailey Southeast Cottage | Published | Closed end wedge, porch recess, thick roof; `southeast-cottage-pass3`. |
| Lower Bailey Northwest Cottage | Published | Hipped roofs, barrel, fence, wash tub; `northwest-cottage-final-v5`. |
| East Bailey South Curtain Lean-to | Worker ready | Chimney, thick roof and walls; `south-shelter-pass2`. |
| East Wall Lean-to Shelter (formerly wall landing) | Worker ready | Closed body and thick shelter roof; `east-landing-worker-v2`. |
| East Bailey North Curtain Timber Shed | Worker ready | Chimney, annex, main shed and doorway recess; `north-shelter-pass3`. |
| Lower Bailey Supply Cart (formerly stair supplies) | Worker ready | Cart, two spoked wheels, barrel and hoops; `east-supplies/accepted`. |
| East Bailey Gatehouse | Worker ready | Closed gate assembly, arch, roof, rounded turret, 7 crenels; `east-bailey-gate/final.blend`. |
| East Bailey East Curtain | Published | 31 crenels and closed support geometry; `east-bailey-east-curtain` recipe. |
| East Hall Exterior Stair | Published | Corrected painted/geometric mismatch to 13 treads; trough refined; `stair-texture-fix/final`. |
| East Bailey West Curtain | Worker ready | 19 crenels; some dark fallback texture remains; `east-bailey-west-worker-v4`. |
| East Bailey Stacked Timber | Worker ready | 51 closed logs with staggered stacking; `stacked-timber-worker` v6. |
| East Bailey Covered Well | Worker ready | Timber canopy, hollow shaft, bucket; `east-well-final-v7`. Ground bucket ghost cleanup ready separately. |
| East Bailey Barrel (formerly supply crate) | Worker ready | Lying barrel with two hoops; verify authored hoop materials survive reprojection before integration. |
| East Bailey Chopping Block (formerly yard prop) | Worker ready | Stump, axe head and handle; `east-bailey-yard-prop-worker2`. |
| Upper Bailey West Curtain | Worker ready | 21 crenels, 13 treads, thick covered roof; `upper-west-worker-v3`. |
| Upper Bailey Gatehouse | Worker ready | Open arch, closed components, 11 physical crenels; closure and interior parts retained; `upper-gatehouse-agent/worker-v2`. |
| Keep to East Hall Bridge | Worker ready | Arch void, deck-height parapet, coping and closed landing; `bridge-worker/worker-v2`. |
| Upper Bailey East Curtain | Worker ready | Closed walk and 14 crenels; `upper-east-curtain-worker1`. |
| Great Keep | Published | 34 total crenels, closed roofs without ground curtains, interior stair corrections. Interior texture/fallback quality still needs scrutiny. |
| East Hall | Published | 19 crenels and architecture corrections; `east-hall-complete-review-v3`. |
| East Watchtower | Worker ready | 7 crenels, curved cap, trimmed shaft and supports; `east-watchtower/checked.blend`. Inherited part 215 still has 54 open edges; not declared manifold. |
| Ground, courtyard levels, slopes and foreground banks | Published | Terrain relief with supported footprints and source projection retained. Not a complete terrain-detail pass. |
| Distant landscape/background | Initial pass integrated | Inferred valley relief; distant geometry remains uncertain. |
| Ground prop ghost cleanup | Worker ready | `derby_ground_cleanup.py`: 1,935 edited pixels, outside-mask identity and PNG round-trip checked; `ground-cleanup-worker/worker-v3.blend`. Requires refined wells before integration. |

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
- `apply_multiview_texture.py` applied its preserved composite to 82 rear-facing
  faces, choosing one visible camera per face. 104 hidden faces were skipped.
  Front-facing faces were not reassigned. No cross-view blending is implemented.
- Rear preview has obvious triangular mismatches/seams. This is an experimental
  publication, not an accepted final texture bake. See
  `sunburst-editor-preview/rendered/view-4-textured.png`.
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
3. Integrate all worker-ready recipes and ground cleanup, preserving the generated
   material where applicable; rerun layered projection and synchronize names.
4. Export/publish the map and all 30 standalone assets together; validate editor
   loading, whole-group/part selection, UVs, fingerprints and reveal metadata.
5. Regenerate full-map reference solid/textured comparisons and update this table
   from actual integration/publication results.

Source art cannot establish unseen geometry uniquely. Record remaining defects
instead of treating a completed worker pass as a perfect model.
