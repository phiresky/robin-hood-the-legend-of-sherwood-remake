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

**Second full-scene refinement pass is active.** The immutable starting snapshot
is `work/derby-refinement/round-2/source.blend`. A fresh grouping review confirmed
34 logical assets and 270 canonical parts; each asset will receive its own worker
in scheduled waves. Six asset candidates have passed root review: Great Keep,
Upper Bailey Gatehouse, South Gatehouse, East Hall turret, East Watchtower and
the revised West Cottage. Terrain/background also has an accepted candidate.
These candidates are combined in `round-2/integration-geometry.blend`, with all
eight recipe scope checks passing (30 building parts plus ground changed).
They are **not yet published or merged into the main working blend**. Keep facade relief,
Hall wall/interior alignment and Watchtower hoist/cage remain follow-up work.
New work is isolated until its source,
solid, textured and scope checks pass. The new mask PNGs are evidence for this
round; previous refinements may be removed or rebuilt. Initial publications
listed below are baselines, not proof of acceptance or completion of this round.

Detailed round-two coverage is in `round-2/status.md`; accepted recipes are in
`round-2/integration.json`. The East Cottage review identified part 52 as a
freestanding open yard trough, not a lean-to. Split it into a named asset after
workers finish against the frozen 34-group catalog. A fresh Postern review also
corrected the old eight-crenel claim: the inherited mesh has two cut crenels and
the artwork supports a sloping damaged rim, not eight invented notches.

All 30 original logical assets have received an individual worker pass. **All initial
passes are now merged and published, with 34 standalone assets after regrouping. The
assets are not finished.** “Published”
means the geometry is included in the latest map export, not that all defects
are resolved. West Cottage's fourth geometry pass and generated texture bake are
now published; the generated hidden details remain inferred.

Latest map publication: `work/derby-refinement/synthesized-ownership-publish-v2/derby.scene.glb`
to `library/scenes/derby-volumes.scene.glb`, with the document fingerprint updated.
It contains 34 groups, 270 canonical parts, 306 meshes, and 154 modeled steps.
Browser acceptance passed: 34 named groups, 270 parts, 14 stair assemblies,
154 steps, 145 reported crenel notches, 30 interior meshes and seven reveal patches.
Real ray picking selected Great Keep as a group first, then its part 146.
All 34 standalone descriptors/models were published from the same scene.
The document migrated six still-authored part memberships; custom grouping and
world transforms are preserved. Focused ownership-migration tests and pipeline
typecheck passed.

Working integration checkpoint: `derby-refinement.blend`. The latest scene-wide
ownership bake is recorded in `synthesized-ownership-publish-v2/reprojection/layers-report.json`:
five layers, 4,468,557 known texels and 18,389,748 unknown texels, now filled where
eligible source donors exist. It retains
explicit authored/generated materials and the cleaned ground. Cottage V4 then
received its own fresh projection and generated texture bake after its geometry edit.
Well ground-ghost cleanup is now applied (1,935 pixels), and 21 mesh names synced.
West Cottage V4 remains the published but rejected shape baseline. The round-two
replacement has a level ridge and eaves, mask-fitted walls and a continuous rear
roof/wall join (3,273 exterior seam samples without daylight). Its union silhouette
IoU is 94.60%; thin fringe residual and a simplified porch remain. Review evidence:
`round-2/assets/derby-lower-west-cottage/modified/`.

Paths below are relative to `level-editor/work/derby-refinement/`.
Reusable geometry recipes are in `level-editor/blender/derby_asset_*.py`.

New integration published: reviewed ownership now has 34 groups (Hall
stairs joined to Hall; detached yard props split out). West Cottage V4, the
Lower East Curtain stair-side masonry fix, rounded Keep turret and corrected
Keep hanging turret are merged into the working blend and editor export. The hanging turret had
incorrectly reached ground level, capturing a roughly 534-pixel background strip;
it now ends at its visible tapered support. Its new body is closed after positional
welding; inherited upper parapet seams remain. Evidence:
`great-keep-second-pass/right-side-before-after.png`.

Patch audit found mission-owned bridge/mechanism animations were omitted by the
base-map exporter. `drawbridge-state-audit/layers.json` now includes seven base
patches plus nine mission records (three structures across three missions), with
initial/transition/applied graphics. The courtyard bridge is canonical part 267,
attached to Upper Bailey Gatehouse; sight changes do not remove its lowered deck.
The courtyard bridge now has 12 physical planks, separate original textures on
opposite sides, two suspension strands and a passage floor. Raised/lowered and
oblique reviews passed; the published pose is raised. GLB metadata carries nine
mission records and 107 deduplicated original animation images. Automatic editor
patch animation, the second bridge's refined geometry, and applied winch debris
geometry remain unfinished. Individual chain links are simplified.

| Asset | Integration | Latest work / remaining review |
| --- | --- | --- |
| South Gatehouse | Second-pass geometry accepted, not published | Rebuilt curved roofs, eave collars and round drums. West silhouette IoU 88.85% to 96.29%; east 86.36% to 94.90%. Narrow roof-sector shading seams remain to inspect. Fresh projection must replace old generated UVs on changed meshes. Prior editor hole was fixed by compacting material-used UV channels. |
| Southwest Postern Tower | Published baseline; second pass active | Old eight-crenel claim was incorrect; fresh inspection found two physical notches. Source-fitted scaffold reconstruction is in progress. |
| Lower Bailey East Curtain | Stair-side fix published | 25 crenels and closed supports. Stair building-012 side faces now use coherent curtain masonry, with tread UVs and geometry unchanged; `stair-cheek-fix/visible-after/side-textured.png`. |
| Lower Bailey West Curtain | Published | 25 crenels, rounded turret, curved roof sectors, 18 stairs; oblique joins remain reviewable. |
| Lower Bailey Well (formerly courtyard prop) | Published | Hollow shaft, iron lifting frame, rope, pulley, bucket; `lower-well-worker-v2`. Printed ground ghost cleanup integrated. |
| Southern Approach Stone | Published | Closed angular outcrop, replacing tent-like wedges; `approach-stone-worker/worker-v2`. |
| Lower Bailey East Cottage | Published | Hipped roof, closed walls, barrel; `lower-east-cottage`. |
| Lower Bailey West Cottage | Second-pass geometry accepted, not published | Main roof/walls rebuilt against masks 9/10; level ridge/eaves and rounded annex roof. Initial candidate had rear daylight gaps and was rejected; revised candidate closes them. IoU 94.60%; simplified porch and thin fringe remain. Generated hidden detail is not geometry evidence. |
| Lower Bailey Southwest Cottage | Published | Thatched eave and damaged-roof recess; `southwest-cottage/reviewed`. |
| Lower Bailey Southeast Cottage | Published | Closed end wedge, porch recess, thick roof; `southeast-cottage-pass3`. |
| Lower Bailey Northwest Cottage | Published | Hipped roofs, barrel, fence, wash tub; `northwest-cottage-final-v5`. |
| East Bailey South Curtain Lean-to | Published | Chimney, thick roof and walls; `south-shelter-pass2`. |
| East Wall Lean-to Shelter (formerly wall landing) | Published | Closed body and thick shelter roof; `east-landing-worker-v2`. |
| East Bailey North Curtain Timber Shed | Published | Chimney, annex, main shed and doorway recess; `north-shelter-pass3`. |
| Lower Bailey Supply Cart (formerly stair supplies) | Published | Cart, two spoked wheels, barrel and hoops; `east-supplies/accepted`. |
| East Bailey Gatehouse | Published | Closed gate assembly, arch, roof, rounded turret, 7 crenels; `east-bailey-gate/final.blend`. |
| East Bailey East Curtain | Published | 31 crenels and closed support geometry; `east-bailey-east-curtain` recipe. |
| East Hall Stone Trough | Published separately; stairs belong to Hall | Stair flight and landing are now within East Hall's group; trough remains separately selectable. Stair geometry retains 13 treads. |
| East Bailey West Curtain | Published | 19 crenels; some dark fallback texture remains; `east-bailey-west-worker-v4`. |
| East Bailey Stacked Timber | Published | 51 closed logs with staggered stacking; `stacked-timber-worker` v6. |
| East Bailey Covered Well | Published | Timber canopy, hollow shaft, bucket; `east-well-final-v7`. Ground bucket ghost cleanup integrated. |
| East Bailey Barrel (formerly supply crate) | Published | Lying barrel with two hoops; verify authored hoop materials survive reprojection before integration. |
| East Bailey Chopping Block (formerly yard prop) | Published | Stump, axe head and handle; `east-bailey-yard-prop-worker2`. |
| Upper Bailey West Curtain | Published | 21 crenels, 13 treads, thick covered roof; `upper-west-worker-v3`. |
| Upper Bailey Gatehouse | Drawbridge correction published | Open arch, 11 crenels, hinged timber bridge replacing erroneous masonry closure on part267. Bridge state artwork and metadata included. Existing inner-wall oblique texture smears remain. |
| Keep to East Hall Bridge | Published | Arch void, deck-height parapet, coping and closed landing; `bridge-worker/worker-v2`. |
| Upper Bailey East Curtain | Published | Closed walk and 14 crenels; `upper-east-curtain-worker1`. |
| Great Keep | Right-side background fix and round turret published; further refinement required | 34 crenels, closed roofs, corrected interior stairs. Four-sided roof replaced by curved round turret with open arched doorway. Hanging right turret no longer reaches ground or captures background. Facade arches/windows, cornices, other tower profiles, inherited parapet seams and hidden geometry still need work. Unaccepted facade experiment excluded. |
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

The user still rejects the cottage's shape. Treat V4 as a rejected geometric
hypothesis for the next pass, not an outline to preserve. Character occlusion
bitmaps provide stronger original-camera silhouette evidence than obstacle
volumes. Derby has 236 masks; masks 9 and 10 cover the cottage annex and main
body/roof, with additional porch/detail masks nearby. Extracted evidence:
`cottage-occlusion-masks/`. These are conditional occluder silhouettes, not
3D depth maps or automatically one complete semantic object per mask. Refit the
cottage from this evidence before another texture generation; replace earlier
geometry wherever necessary. The reusable worker instructions now say this explicitly.

The hackable datadir converter now exports these masks automatically. Production
Derby evidence is at `datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.d/masks/`
(repository-relative): 236 lossless PNGs plus `manifest.json`, including the main
cottage silhouette `000010.png`. Backfill completed for all nine maps (3,027
masks); all Derby PNG pixels match the independent extraction and level JSON is
unchanged. The reusable projection constraint loader accepts this inventory, but
requires reviewed object/mask associations and matching source-image hashes.
No automatic Derby associations have been enabled yet.

Initial association review: `mask-association-review/review.md` and `review.json`.
Cottage mask10 guides the main assembly55/56, mask9 the annex54; masks7/8 need
porch-submesh review. Masks22–24 reveal a detached cottage yard fence still missing
from the modeled catalog. At Upper Bailey Gatehouse, revealed mask227 matches
stair265, but floor-associated masks220/223 are composite facade/terrace masks,
not exclusive floor ownership. Reveal003 switches global217–220 to221–229.
Bridge masks230–232 describe foreground occlusion, not a complete deck silhouette.
Use masks alongside geometric visibility, never in place of it.

Editor appearance follow-up is published: optional synthesized hidden
surfaces and smooth texture filtering. The updated map and all 34 standalone
assets are now published. The editor defaults to smooth textures (linear,
trilinear mipmaps and anisotropy), with persisted View switches for smoothing and
synthesized hidden surfaces. Source-only worker evidence retains neutral unknown
surfaces; inferred fill never becomes projection evidence.

The new texture refresh produced 170 example-based synthesis tiles across five
projection layers, with 414 thin-donor mirrored fallbacks. Geometry, world
transforms, names and groups match the previous checkpoint exactly; all six
protected authored/AI texture hashes are unchanged. There are 4,468,557 observed
texels and 18,389,748 unobserved texels; 70,364 cottage texels remain neutral
because no eligible source donor exists on those meshes. Ownership is recorded
separately from RGB in opaque exported materials. Tests cover source RGB
preservation, GPU alpha-zero RGB preservation, source-only display, filtering,
and unchanged geometry. Final published-map browser verification passed:
301 textures, 300 synthesized materials, 4,815 changed samples when toggled,
unchanged opacity, and exact image restoration when toggled back. Oblique
comparisons are `verification/synthesis-oblique-on.png` and
`verification/synthesis-oblique-off.png`. Visual inspection confirms filled
back surfaces; sparse donors still produce conspicuous color bands and repeated
patterns, so this is a configurable preview rather than finished texture art.
Saved model: `derby-refinement.blend`; previous checkpoint:
`derby-refinement-before-synthesis.blend`.

Latest reported defects corrected and published:
- Cottage ridge is level at 121.53 and long eaves at 87. The local front lip drops
  to 70; its hip apex matches source pixel (607,1919). Eight components are closed
  and nondegenerate. Texture bake retained geometry exactly.
- Gatehouse double-assigned crenellations were caused by interior projection
  omitting retained exterior battlements from its occluders. Corrected layer rules
  plus per-texel first-hit depth checks fix that assignment. Independently checked:
  all 104 previously blocked roof samples now unknown, zero leaked textures;
  276 of 280 visible samples remain textured, four are boundary texels.
- Old projected fallback textures were an additional general problem. Final
  ownership bakes replace hidden texels with neutral shading instead of retaining
  those atlases. Other partially cut-away facades retain explicit audit limitations
  in `interior_layers.py`; their face-level visibility still needs further review.

Reusable tooling is implemented: a complete scene grouping review precedes per-asset agent
workspaces. Each workspace receives a model, instructions, original context crop,
and eight-view solid/source-only textured sheets. `modified/` repeats the layout
with fixed cameras and fresh projection; synthesized textures are excluded from
reference evidence. The worker model itself now uses source-only materials too.
The grouping audit moved Hall stairs into the Hall and detached yard props into
independent groups. Tested workspace: `agent-workspaces-v4/derby-lower-west-cottage`.
Its `input/` and `modified/` have identical layouts, cameras, context and known-source
masks. Six solid-render pixels differ by at most 4/255; geometry is unchanged.
All 2,840 active faces use source-only materials. Ownership tests reject outside geometry, visibility
and source-ID changes. The current round-two packets and actual coverage are
tracked in `round-2/status.md`; not all 34 new refinement passes are complete.
Worker instructions explicitly encourage detail close-ups, additional angles and
section views in `inspection/`. New eight-view packets fit evaluated mesh vertices
per angle with four percent padding instead of a shared worst-angle bounding-box
scale. Modified renders retain their input cameras for direct comparison.
Mission-aware review sources were separately verified in `drawbridge-workspace-review`.

West Cottage V4 was published with fresh source projection,
high-quality no-mask generation and atlas bake in
`west-cottage-texture-baked-v4/worker.blend`; see `eight-views.png` there.
Thin neutral base strips remain in some generated side/rear views; hidden details
are inferred. Its shape was subsequently rejected; the accepted round-two geometry
must receive fresh projection before another texture-generation pass.

1. Keep the reviewed gatehouse shape while resolving texture artifacts. Export UV
   compaction fixed the editor-only hole; source and exported roof triangles match.
   Do not reshape the roof to compensate for a texture bug. Internal nonplanar caps
   remain a separate potential cleanup, not a demonstrated cause of this defect.
2. Improve generated rear texture placement and verify the original-camera render
   remains unchanged. Do not infer texture correctness from the generated sheet.
3. Integrate the accepted round-two West Cottage replacement, clearing stale
   authored UV preservation on its changed parts before fresh projection.
4. After subsequent changes, export/publish map and standalone assets together and
   repeat editor loading, selection, UV and reveal checks as appropriate. Initial
   combined publication and checks are complete.
5. Full-map comparisons for the combined geometry are in
   `ownership-level-roof-publish/full-map/reference-{solid,textured}.png`; refreshed after
   the latest cottage, Keep and bridge changes. Keep this table current.

Source art cannot establish unseen geometry uniquely. Record remaining defects
instead of treating a completed worker pass as a perfect model.
