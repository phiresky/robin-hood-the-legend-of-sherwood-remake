# Derby asset refinement review

Every logical asset receives an independent geometry and projection review.
Reviewed does not mean finished: record remaining defects and uncertain geometry.
Workers use separate Blender copies; the primary session integrates reviewed
recipes, reruns layered projection, and publishes the map and standalone assets.

Blender concurrency is provisionally nine coordinated jobs, increased from three
after host measurements on 2026-09-20: 16 logical CPUs, 30.65 GiB RAM, roughly
20 GiB available, and sampled workers using 1–2 GiB and about one CPU core each.
This is capacity headroom evidence, not a nine-job throughput benchmark. New jobs
use `--threads 2`; ready work starts without mandatory load delays. High host I/O
pressure did not establish device saturation: a subsequent five-second NVMe
sample showed 12.3% busy time, 14.9 MiB/s combined throughput and 3.68 ms average
read/write completion latency. Pause new launches below 6 GiB available memory
or measured swapping/device latency causing sustained slowdowns; do not kill
active work. Recheck with
`python3 level-editor/blender/measure_blender_resources.py --seconds 10` on the
host, since sandbox process isolation hides other workers. Worker instructions
and coordinator scheduling use the same provisional cap.

Latest user approval: "both approved" applies to East Watchtower `ec675d3ef`
(`inspection/masked-approval-v1`) and the South Gate east-roof correction
`88747d706` (`derby-south-gatehouse-candidate-mask63-v2/inspection/final`).
Generate with two images, high quality, no API mask; preserve exact input,
lighting, ownership evidence, raw and source-preserved outputs. Gate approval
covers revised roof parts 013–016; reproject and bake against their new geometry
rather than reuse old generated roof UVs. Other approved gate parts stay intact.

Earlier user review: Southwest Postern `ca3fcb885` (`inspection/approval-v3`) and
Northwest Cottage `c2fd98fc2` (`modified/`) are approved for the two-image,
high-quality Sunburst pass without an API mask. Retain raw and source-preserved
outputs and exact input/provenance records. Cottage ridge projection stretching
and remaining silhouette discrepancies were disclosed in this review.
Lower West Curtain `ed9f7782a` must first be split into smaller logical assets
for editing and texture generation, as requested by the user; its generation
is on hold. The split must preserve transforms, complete part ownership and
mask constraints, and be reflected in reusable map/individual-asset exports.

Mask authority correction: earlier mask inspection and selected export constraints
did not enforce ownership in worker previews. A successful mesh visibility test
or high silhouette IoU is insufficient to accept source pixels. Every handoff must
report reviewed mask IDs, source state/hash, inclusion/exclusion constraints,
rejected samples and any accepted foreground contamination. Carry this evidence
through source-only previews, generated-texture baking and export; configured
constraints must not be silently dropped. Masks establish reviewed pixel
membership, not recoverable 3D depth. Unconstrained assets must be labeled as such.
Southeast Cottage exposed this gap: its frozen scene accepted 639 native tower
pixels. The corrected masked preview accepts zero, with identical solid geometry.
Latest published South Gate geometry is audited separately from that old context.

**User approval is required before each GPT Sunburst texture-fill pass.** Show
the model's current solid and source-only textured views, disclose remaining
geometry issues, and record explicit approval for that revision. Technical
acceptance by a worker or coordinator is not user approval. Further geometry
changes invalidate the approved revision and require another review. Do not
start a Sunburst request while approval is pending.

Current approval queue: West Cottage's pointed/flared rear roof was rejected.
A straight-gable correction improved it, but the user requested coherent slanted
front/rear facades. The latest candidate removes the V-shaped rear wall and uses
shared end planes with 2.86-degree inward batter. After seeing both eight-view
`facade-review/` sheets, the user approved this geometry (`d01d438af`):
"ok good enough for now". Prepare the unchanged geometry with fixed-world-sun
lighting and a separate solid reference. Its two-image Sunburst generation is
complete in `round-2/sunburst-cottage-approved/`; application is pending.
The user also approved all three shown candidates for texture filling:
Lower East Curtain and stairs (`01badfdfa`, `approval/`), East Cottage barrel
(`dd4075b59`, `approval/`), and Northwest Cottage barrel (`2a38c34ee`,
`modified/`): "approve all three". Their fixed-world-sun textured and solid
sheets are the approved references. High-quality, no-API-mask, two-image
Sunburst passes completed successfully; generated textures are not yet published.
Their results are under `round-2/sunburst-<asset-id>-approved/` in
`generation-short-no-mask-with-lighting/`. All three preserved composites retain
the protected source pixels exactly. The user reviewed the Northwest barrel raw
output positively; its future use versus source-preserved pixels remains open.
East Cottage geometry (`6bf42a049`) was also explicitly approved for texture
fill after its `approval-fixed-sun/` solid and source-textured sheets were shown:
"Approve texture fill". Its two-image, high-quality, no-mask generation is being
prepared in `round-2/sunburst-east-cottage-approved/`; application is pending.
East Bailey Gatehouse candidate `d0b5d3d77` failed user review of the gate arch.
The roof/support pass retained the inherited arch (083); that opening needs a
dedicated source-image and geometry correction, including jambs and passage
depth. Do not generate gate textures until the corrected arch is shown and
approved. East Bailey East Curtain (`a70eeef9e`) was approved separately after
its solid/source-textured sheets were shown: "the curtain looks good". Its
two-image, high-quality, no-mask texture fill is queued in
`round-2/sunburst-east-bailey-east-curtain-approved/`. This is distinct from the
already-generated Lower Bailey East Curtain.
The user approved both lean-tos for two-image texture filling: East Wall Lean-to
(`54589c289`, `inspection/approval/`) and South Curtain Lean-to (`82ea176b2`,
`modified/`), with "Approve both". Geometry is frozen. High-quality no-mask
generation is assigned; retain raw and source-preserved alternatives in
`sunburst-east-wall-lean-to-approved/` and `sunburst-south-curtain-lean-to-approved/`.
Southwest Cottage (`21273f62d`) and Keep–Hall Bridge (`2d3cc7425`) were approved
for texture filling after their solid/source-textured eight-view review.
Southeast Cottage (`ea9db23ca`) was rejected because foreground tower artwork
is projected onto cottage surfaces. No AI pass is authorized for that candidate.
Review native cottage mask 0 against foreground tower mask 63, state and depth
ordering; apply matching source-acceptance constraints to both baking and worker
review images. Correct ownership rather than distorting geometry or generating
over contaminated evidence. Show corrected source-only views before approval.

Corrected East Bailey Gatehouse arch `b51aebb25` was explicitly approved after
renewed closeup review: "Approve texture fill". Its previous crown was about 25
source pixels too high; the new arch uses rounded shoulders and continuous jamb
depth. Its high-quality two-image Sunburst fill, with no API mask, is queued in
`round-2/sunburst-east-bailey-gate-approved/`; geometry is frozen for this pass.
West Cottage and Lower East Curtain source-protected texture bakes passed root
visual review and are queued for publication-3. The two cottage barrel bakes are
held: despite topology/source-preservation checks passing, coarse original source
pixels stretch conspicuously beside generated surfaces in alternate views. Raw
outputs remain retained as an alternative; no choice to replace known pixels has
been made. Do not publish those barrel texture candidates as finished results.

**Retain both texture alternatives.** Keep `generated-raw.png` and
`generated-preserved.png`, the exact input and solid lighting reference,
camera/ownership manifests, approval record, prompt/settings and cached API
response. Do not overwrite or delete either result when baking, publishing or
running another experiment; put new generations in a new directory. The user
explicitly wants to decide later whether to use raw generated pixels instead of
preserved source pixels. The raw Northwest barrel output SHA-256 is
`be6e594f7c16bbf116e06353397732ed9b725494d2b9272f9b984faf736c43db`.
The user approved South Gatehouse's corrected fixed-world-sun input explicitly.
Its Sunburst pass completed with `gpt-image-2.5-sunburst`, quality `high`, no API
mask, and the prompt sentence requiring the supplied shading. Exact approved
input SHA: `1954359756442205b43e6e10566955ffaec8c9d84f96a9817610fbc1a5e119c6`.
Artifacts: `round-2/sunburst-gate-approved/`; raw and protected outputs were shown.
The user then requested a second test with the pure-gray sheet as a separate
lighting reference, and explicitly selected its result for the model:
`generation-short-no-mask-with-lighting/generated-raw.png`. Use that selected
two-image result, not the initially approved single-image result.
The raw result changes existing artwork; the protected composite preserves it
exactly and fills 330,969 unknown pixels. Projection back onto the approved model
and seam review passed. The selected two-image result is now published in the
map and standalone gate asset; all 17 generated materials were verified in the
editor, with 295,069 source samples protected exactly during the model bake.
Keep, East Hall, East Watchtower and Postern
have substantial geometry follow-ups and are not ready for texture generation.

For each asset check every component against source artwork, reference and two
oblique views in textured and solid modes; inspect silhouette, roof/wall joins,
openings, stairs, surface orientation, texture ownership, hidden-face fallback,
group/part identity, repeated execution, and exported geometry/UVs. Keep original
collision data separate from refined rendering geometry.

## Current integration state — 2026-09-20

User requested splitting Great Keep into 2–4 logical components. The four-way
proposal in `round-2/keep-component-proposal.json` assigns every existing part
exactly once: West Tower (including its revealed room), Main Hall (including its
interior), North Tower, and Central Turret/Gallery. Connecting geometry needs
spatial review before catalog/editor migration. Existing frozen worker catalogs
remain intact during this preparation; the split is not published yet.

Interior evidence audit: all 34 worker folders include original covered/revealed
composites, individual exterior patch PNGs and alpha masks, `layers.json`, and
mission-state images. Keep review projection layers explicitly use `revealed.png`
for interior receivers. Availability does not establish that every state has
been reviewed. Worker instructions now require named patch/state inspection and
covered/revealed context plus solid/source-textured closeups before declaring an
interior building reviewed; exterior eight-view sheets alone are insufficient.

**Second full-scene refinement pass is active.** The immutable starting snapshot
is `work/derby-refinement/round-2/source.blend`. A fresh grouping review confirmed
34 logical assets and 270 canonical parts; each asset will receive its own worker
in scheduled waves. Twelve reviewed recipes are now integrated and published:
terrain/background, rock relief, Great Keep, Upper Bailey Gatehouse, South
Gatehouse, East Hall turret, East Watchtower, Southwest Postern, Lower East
Curtain, Lower West Curtain, and the two detached cottage barrels. The source
and geometry checks are recorded in `round-2/integration.json` and the staged
publication evidence. **Publication 3 also includes the approved West Cottage
facade/roof correction and source-protected texture fill.** Keep facade
relief, Hall wall/interior alignment and Watchtower hoist/cage remain follow-up work.
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
are resolved. West Cottage now includes the approved `d01d438af` geometry and its
new generated bake. Generated hidden details remain inferred.

Latest map publication: `work/derby-refinement/round-2/publication-3/derby.scene.glb`
to `library/scenes/derby-volumes.scene.glb`, with the document fingerprint updated.
It contains 34 groups, 270 canonical parts, 308 meshes, and 155 modeled steps.
Browser acceptance passed: 34 named groups, 270 parts, 14 stair assemblies,
155 steps, 145 reported crenel notches, 30 interior meshes and seven reveal patches.
Real ray picking selected Great Keep as a group first, then its part 146.
All 34 standalone descriptors/models were published from the same scene and
verified byte-identical to staging. No group or part migration was needed in this
publication. Custom grouping and world transforms are preserved.

The user's selected **two-image South Gatehouse Sunburst result is published**.
All 17 gate materials carry generated SHA
`be9bdd585ad7cc53d76733d8bdba4f2bbdb526ea2a45ddc569f93cbba16d6eea`,
verified in the actual browser-loaded GLB. Only source-hidden texels were filled;
295,069 source samples were protected exactly. Camera overlap and source-boundary
tone reconciliation reduce roof seams without changing protected artwork or
geometry. A subsequent exterior reprojection preserves all 7,161 gate faces.
Evidence: `round-2/gate-approved-two-image-texture-bake/` and
`round-2/publication-3/{stage,publication,browser-result}.json`.

Publication 3 imports only the approved West Cottage geometry and its eight
texture materials, plus six source-protected Lower East Curtain texture materials.
The browser verifies their selected generated hashes (`0d79dab5c4c06363…` and
`fbe664a15896bd44…`). All 733 outside mesh material/UV signatures and all 17
Gate atlas pixel buffers remain unchanged. The two barrel texture candidates
remain unpublished because their source/AI transitions need further work.

Working integration checkpoint: `derby-refinement.blend`. The latest scene-wide
ownership bake is recorded in `round-2/publication-2/reprojection/layers-report.json`:
five layers, 4,526,903 known texels and 18,853,940 unknown texels before the approved
gate texture handoff. Eligible hidden areas receive optional source-based synthesis.
The imported gate atlas remains independent of that display toggle. The checkpoint
backup is `derby-refinement-before-round2-publication3.blend`; the prior map/document
backup is `library/scenes/backups/77a204cd9e19a1f6/`. Publication 3 retains the
existing scene-wide projections, then imports the separately verified Cottage
and Curtain source-protected atlases with exact geometry guards.

Updated full-Derby 1920×2752 renders:
`round-2/publication-3/full-map/reference-solid.png` and
`round-2/publication-3/full-map/reference-textured.png`. These retain visible
unfinished Keep/Hall details and other recorded limitations; publication is not
final asset acceptance.

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
| South Gatehouse | User-approved geometry and selected two-image textures published | Rebuilt curved roofs, eave collars and round drums. West silhouette IoU 88.85% to 96.29%; east 86.36% to 94.90%. Fresh source projection and generated hidden-surface bake passed geometry/source-preservation checks; all 17 selected materials verified in the editor. Prior editor hole was fixed by compacting material-used UV channels. |
| Southwest Postern Tower | Published baseline; second pass active | Old eight-crenel claim was incorrect; fresh inspection found two physical notches. Source-fitted scaffold reconstruction is in progress. |
| Lower Bailey East Curtain | Stair-side fix published | 25 crenels and closed supports. Stair building-012 side faces now use coherent curtain masonry, with tread UVs and geometry unchanged; `stair-cheek-fix/visible-after/side-textured.png`. |
| Lower Bailey West Curtain | Published | 25 crenels, rounded turret, curved roof sectors, 18 stairs; oblique joins remain reviewable. |
| Lower Bailey Well (formerly courtyard prop) | Published | Hollow shaft, iron lifting frame, rope, pulley, bucket; `lower-well-worker-v2`. Printed ground ghost cleanup integrated. |
| Southern Approach Stone | Published | Closed angular outcrop, replacing tent-like wedges; `approach-stone-worker/worker-v2`. |
| Lower Bailey East Cottage | Published | Hipped roof, closed walls, barrel; `lower-east-cottage`. |
| Lower Bailey West Cottage | Corrected geometry user-approved; new texture generation complete | Straight rear gable and coherent inward-leaning facade planes replace the rejected flared roof. User approved d01d438af after eight-view review. High-quality two-image Sunburst result retained alongside source-preserved alternative; new geometry/texture application pending. |
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
