# York grouping pass

This is the first refinement step: freeze source evidence, inspect the whole map,
and assign the reconstructed surfaces to named logical assets. The reviewed
catalog is `../../refinement/catalogs/york.json`; `ownership.txt` is its editable
recipe. It replaces 460 proximity groups with 252 named groups plus background
terrain. All 972 visible source records and nine records without meshes are
accounted for. Distinct adjoining buildings are separate assets, including attached towers
and gatehouses. Cathedral towers and the north precinct hall are separate from
the nave. The courtyard rear curtain wall owns 760, 761 and battlements 772;
the east lodge owns 759, west lodge 771, and stairs 812. `building-review.json` records the subdivisions of the first pass.

Outputs are local under `../../work/york-refinement/`:

- `baseline/`: immutable scene, native obstacle data, artwork, masks and hashes.
- `inventory/inventory.json`: complete imported mesh and patch inventory.
- `grouped/york-grouped.blend`: named asset parents and retained source identities.
- `grouped/validation.json` and `partition-verification.json`: preservation checks.
- `grounding/york-grounded.blend`: subsequent terrain-trimmed asset scene.
- `grounding/report.json`, `verification.json` and `coverage-audit.json`: cut
  inventory, retained-surface/UV checks and complete-scene burial audit.
- `review/index.html`: shared interactive grouping review, with approve/request
  buttons, original context, selected-part overlay and two geometry views.
- `review/inspection.html`: compact map overviews and direct staged model links.
- `review/evidence.json`: exact image/report/model revisions shown for decisions.
- `grouping-decisions.json`: explicit submitted grouping decisions, separate from
  geometry and texture approvals.
- `grouping-review.json`: exact reviewed catalog/inventory hashes and scope.
- `state-ownership.json`: native patch associations and sprite-only asset inventory.
- `stage/york.rhlos-map.json`: staged map with individual reusable assets in
  `stage/map-assets/3d-assets/`; the live library is not modified.

## Scope and uncertainties

Grouping preserves the reconstruction's existing geometry and textures. The
subsequent grounding pass removes buried surfaces as described below. Missing
walls, floating sections, depth inaccuracies and unseen textures need the next
refinement step. Painted vegetation and decorative details remain part of the
background or building textures, not newly created individual models. Static
coverage does not imply complete animated state or scenery geometry.

Source 650 spans five market-front houses and is now a separate shared occlusion
volume. Its five historical component selectors remain together, preserving their
complete surface union without adding caps. The complete original is retained
hidden. The castle hall and east round tower also share two surfaces
(sources 769 and 795), split along their visible junction. The continuation
through their hidden intersection is inferred. Surface area and UV interpolation
are checked against all three retained originals.

Some native volumes have displaced depth. Source artwork locates source 67 on
the northeast square watchtower ledge; it is owned by that tower.
The north precinct boundary includes clipped low boundary proxies 870–871.
These decisions assign ownership without pretending to repair their shape.

### Second partial review and remaining-map audit

`grouping-audit-round-2.json` records the 131 remaining assets inspected and 46 source ownership
corrections following the 95 submitted decisions. The resulting catalog has
248 named assets plus terrain: 40 existing assets changed, ten were added, and
13 obsolete groups were retired. All 199 other geometry records remain identical.
The attached east-riverside round tower is merged with its gatehouse as explicitly
requested in this review; the general independent-tower policy still applies elsewhere.

Corrections restore misplaced house bodies and roof halves, separate courtyard
and gate walls, attach walkway landings to their bastions, and combine nearby
market displays and tavern furniture. `review-partitions.json` adds eight shared
surface cuts at structural boundaries. Together with the three earlier sources,
eleven partitioned sources retain their complete surface union and UV mapping.
The market southeast narrow house has no lower-storey source mesh: volume 971
follows canopy 962 and belongs to the stall. Its review card explicitly identifies
this remaining geometry limitation.

The previous delivered gallery is frozen in `review-v8/`; its grouped scene,
grounded scene, library and catalog are under `review-round-2/before/`.
`verify_regrouping.py --before level-editor/work/york-refinement/review-v8/geometry.json`
checks unchanged assets and every unsplit moved component.
The floor audit uses the newly grouped baseline:
`audit_grounding.py --before level-editor/work/york-refinement/review-round-2/ungrounded/geometry.json`.
Generate that baseline with `review_geometry.py -- --geometry-only --output
level-editor/work/york-refinement/review-round-2/ungrounded` before grounding.
Changed groupings return to the pending gallery; submitted approvals remain
valid only for matching asset evidence.

### Third partial review: stairs and wall walks

`grouping-audit-round-3.json` records 91 further decisions and three source
transfers. The detached upper-east stair (242) becomes its own asset. The south
gate east wall gains its deck/support (552) and access ramp (578) from the
neighbouring western curtain wall. Three existing groups change and one is
added; all 246 other geometry records remain identical. All 988 retained
components preserve their topology and world positions within float32 tolerance.

The eastern city curtain already includes its deck (565) behind parapet 566.
Its alternate preview now faces the rear so that deck is visible; the first
preview remains the game camera. The precinct south wall, southeast bastion
and southwest wall adjoin the broad raised precinct terrain (087). Their floor
is continuous with that plateau, rather than an independently modeled wall
walk. The two pending precinct cards explain this; the southwest wall retains
its submitted approval and unchanged evidence.

The preceding gallery is frozen in `review-v9/`; scenes, catalog and export are
under `review-round-3/before/`. Run `verify_regrouping.py` with `--before
level-editor/work/york-refinement/review-v9/geometry.json`, `--audit
level-editor/blender/york/grouping-audit-round-3.json` and `--output
level-editor/work/york-refinement/review-round-3/preservation.json`.
Gallery notes accumulate the audit records, and per-asset alternate camera
settings come from their `view_overrides` entries. To prepare previews during
export, run `build_gallery.py --previews-only`; after export succeeds, run
`build_grouping_review.py` to validate the evidence and refresh the pending gallery.

### Fourth partial review: roof halves, frontage and sloping floors

`grouping-audit-round-4.json` records 65 further decisions and twelve component
transfers. Roof halves 499/500 and eave 504 belong to one gatehouse; its west
boundary wall 523 is separate. Courtyard wall 517 leaves the low house. Roofs
648/649 form a separate rear building, and doorstep 602 returns to the narrow
timber house. Source 650 is an opaque, non-solid, non-selectable native volume
across the frontage; keeping it separate avoids presenting its sloping top as
part of five individual buildings. Its coarse shape remains geometry-refinement
work, and its native flags are preserved.

The adjoining lane 088 rises rather than staying at height 90. The reviewed
floor continuations use its measured plane, clamped below at the lower street.
The plane matches all six native lane heights within 0.00014 game units. Apply
it only beneath the named frontage and lane assets; hidden continuation remains
an explicit inference. Stair 106 now loses its support below the lower landing
at height 50. Supports with an explicit floor are trimmed only by that floor,
never by their own support volume. Verification checks every retained vertex
against the local sloping floor, together with surface area and UV preservation.

The previous gallery is `review-v10/`; scenes, catalog and export are under
`review-round-4/before/`. There are 237 identical outside-asset geometry records,
12 changed existing assets, four added assets and one retired asset. The preservation
check covers 933 unchanged components; the floor verifier covers the 55 explicitly
regrounded components. Use the same audit commands as round 3 with the round-4
audit/output paths and `review-v10/geometry.json` as the previous geometry.

### Fifth partial review: outer-east return walkway

`grouping-audit-round-5.json` records 25 further decisions. The passage's return
walkway was still assigned to the middle bastion. Source 244 now has three
components: the unchanged upper curtain-wall deck, the passage return and the
bastion deck. The new boundary follows the bastion entrance between the native
inner and upper neck junctions. Support pier 239 also returns to the passage.
Nonparallel boundaries use explicit halfspace cuts in `review-partitions.json`;
the existing parallel-strip partitions keep their previous behavior.

Both changed cards include a reverse view exposing the walkway, alongside the
game-camera view. The previous gallery is `review-v11/`; scenes, catalog and
export are under `review-round-5/before/`. Verification preserves all 251 other
asset geometry records and 987 components exactly. The repartitioned source is
checked separately for surface coverage, area and UV preservation. Use the
round-5 audit/output paths and `review-v11/geometry.json` with
`verify_regrouping.py`. Only the passage and middle bastion require renewed
grouping review; the asset count remains 252 plus terrain.


Base patches and Fog mission doors are inventoried separately. Door receiver
ownership and interior/exterior projection receivers still require authored
review before layered projection. No geometry or texture approval is implied by
the static grouping review.

## Terrain grounding

`ground_assets.py` reads the frozen grouped scene and subtracts the volumes below
the reviewed terrain, ramp and raised-lane surfaces from each asset. It trims
477 component meshes across 177 assets. All 252 named assets and terrain remain.
Cuts follow both the support footprint and its sloping height, preserving exposed
lower walls at terrace edges. Bridge decks and roofs are not solid-ground cutters.
The eleven support sources are listed explicitly in the recipe and its report.

Terrain footprints often stop at a building frontage instead of continuing
under its foundation. Footprint-only clipping therefore missed the courtyard
lodges and left partial foundations under the stairs and other edge assets.
`floor-contacts.json` records 38 reviewed floor continuations, with the adjoining
support, game height and source-artwork rationale for each. `floor_contacts.py`
applies each continuation only to its named asset. Lower streets, exposed
retaining walls, bridges and buildings with visible lower facades retain their
lower geometry. These hidden floor continuations are explicit inferences;
they do not add visible terrain or alter native gameplay obstacles.

The floor-contact correction is compared against `review-v7/geometry.json`;
all 228 other geometry records are identical. Before/after source-camera
comparisons and the map-wide candidate survey are in `grounding-investigation/`.
`grounding/outside-preservation.json` records the comparison, and the coverage
audit now checks every vertex against each reviewed floor independently of
the terrain footprint. The previous delivered scene and export are archived
in `grounding-v3/` and `stage-v6/`.

Clipping interpolates every UV channel and retains material assignments. No caps
or replacement textures are added. Untrimmed meshes remain in a hidden reference
collection, and the grouped input file is unchanged. Asset origins move to the
lowest retained contact point; visible world geometry stays in place. The export
localizes mesh positions and obstacle heights together and verifies map placement.

The gallery and staged export now use this grounded scene. The previous delivered
gallery and library are preserved in `review-v4/` and `stage-v4/`. Verification
checks retained surfaces/UVs, accounts for removed area, tests every removed
polygon against its terrain support, and audits the full scene for buried surface
samples. Boundary comparisons allow 0.003 scene units for float32 rounding; the
ground base includes 0.1 units of reconstruction quantization tolerance.

## Reproduction

Run from the repository root, using the shared render-slot pool for Blender.
`setup_scene.py` requires the archived reconstruction in the path it records and
refuses an existing baseline. `group_scene.py` refuses an existing grouped blend.
Preserve earlier output revisions before rerunning either.

```sh
node level-editor/pipeline/src/export-interior-layers.ts york level-editor/work/york-refinement/source-states-complete
blender --background --python level-editor/blender/york/setup_scene.py
python3 level-editor/blender/york/survey.py
python3 level-editor/blender/york/build_catalog.py
blender --background --python level-editor/blender/york/group_scene.py
blender --background --python level-editor/blender/york/verify_partition.py
blender --background --python level-editor/blender/york/review_geometry.py
blender --background --python level-editor/blender/york/export_grouped.py
python3 level-editor/blender/york/build_gallery.py
```

Review all source and geometry cards before writing `grouping-review.json`.
Changing the catalog invalidates that review. Geometry workers and texture
synthesis are separate subsequent steps in the shared refinement procedure.

For the subsequent grounding pass, preserve the existing `review/` and `stage/`
outputs first (`audit_grounding.py` uses the delivered `review-v4/` as its baseline):

```sh
blender --background --python-exit-code 1 --python level-editor/blender/york/ground_assets.py
blender --background --python-exit-code 1 --python level-editor/blender/york/verify_grounding.py
blender --background --python-exit-code 1 --python level-editor/blender/york/review_geometry.py -- --grounded
python3 level-editor/blender/york/audit_grounding.py
blender --background --python-exit-code 1 --python level-editor/blender/york/export_grouped.py -- --grounded
python3 level-editor/blender/york/build_gallery.py
python3 level-editor/blender/york/verify_grounded_export.py
```

`ground_assets.py` refuses to overwrite an existing grounded blend. Preserve the
previous grounding directory before rerunning it.

The first model preview uses the game camera: orthographic, 0° yaw and 35°
elevation, fitted to the asset. The second uses an east oblique view (40° yaw
and elevation). Previews use a per-pixel depth buffer: sorting whole triangles
by their centers can incorrectly show hidden faces across roofs and walls.

## Grouping review controls

The shared gallery builder runs in `review_kind: grouping` mode. **Approve grouping** confirms part ownership and naming only. **Request changes** and the
feedback field collect corrections. Drafts are saved in the browser by stable
asset ID and exact revision; **Copy review results** produces text to paste into
chat. Nothing is submitted or approved merely by opening the page.

After the user supplies that text, save it unchanged and run:

```sh
python3 level-editor/blender/york/record_grouping_feedback.py <user-feedback.txt>
python3 level-editor/blender/york/build_grouping_review.py
```

Matching approved groupings disappear from the pending gallery. Changed revisions
remain pending, and previous galleries/evidence remain in `review/history/`.
A solid-preview-only correction preserves an explicit grouping approval only
when the name, notes, ownership/model/geometry report, validation and all source
images still match. `review/grouping-preview-updates.json` records those links
to the exact original approval; edits to assets or source evidence invalidate it.
Grouping decisions never authorize texture synthesis or final geometry publication.
The later geometry review still requires the procedure's complete eight-view
solid/source-textured packets. Browser control verification is available with
`node level-editor/blender/york/verify_gallery.mjs` (using an isolated profile).

## Installing the approved grouping milestone

The completed grouping review covers 252 named assets plus terrain. Installation
is a separate, explicitly requested library update; it does not mark geometry,
textures or missing state receivers fully refined. `publication-grouping-01/`
holds the frozen reviewed exports, browser derivatives, audit and rollback data.
The map retains the existing camera, bounds, provenance and mission metadata.
The stage was installed after the complete editor audit passed: 252 group and
988 part selections, all 252 insertable palette assets, duplication, undo/redo,
save and full-page reload. `installation.json` records the installed bytes and
backup; `post-install-verification.json` records index and outside-file checks.
York remains WIP: choose **York** or **All levels** in the palette's source filter.

## Geometry refinement pass

The subsequent pass is isolated under `geometry-pass-01/`. It retains the
installed grouping milestone and its review history. `progress.json` separates
the 252 grouped assets from geometry approval, texture completion, state
receivers and scenery. All six native mask layers (0, 1, 2, 3, 4 and 6) have an
overview in `mask-depth-overview.png`; individual masks still require local
ownership review. Forty existing asset bounds touch the map boundary and need
complete-object continuation checks. The animation backlog records 14 candles,
17 water effects, three torches, 20 smoke effects and six birds; this inventory
does not establish animation or rendering parity.

`prepare_geometry.py` prepares explicitly named baseline workers using frozen
shared helpers and the machine-wide render pool. It checks the current catalog
against the grounded scene before writing a new ownership reconciliation. The
older grouping review is retained unchanged. Initial packets are diagnostic:
they do not imply refined geometry, reviewed state receivers, calibrated York
lighting or texture approval.

The first reconstruction recipe is `refine_narrow_house.py`. Its native upper
mask 217 excludes the lower walls, so an independently traced lower-house
domain is required for source-completeness review. The recipe adds the missing
lower body, an upper-storey overhang and a closed thick roof. Rear surfaces and
the hidden foundation footprint are explicit inferences. `inspect_geometry.py`
reopens the saved result, renders eight views fitted to the complete object,
and adds native-camera isolated and neighborhood comparisons. These supplements
preserve the initial packet's fixed cameras. Candidates remain private until
source coverage, terrain contact, actual materials and adjoining receivers have
been reviewed.

Generate browser derivatives with `refinement/blender/lossy_assets.py refresh`
against the publication stage's `map-assets/3d-assets`, then prepare a private
editor audit with `prepare_publication_browser.py --map york --document
<stage>/york.rhlos-map.json`. The explicit staged document is appropriate only
after checking that live York still matches the frozen placement baseline.
Run the full `browser/verify_publication.mjs` audit, inspect its screenshots and
retain its config and result under `<stage>/browser/`.
York's 988 part-selection checks need an extended `audit_timeout_ms` budget;
the publication audit selects **All levels** so WIP assets remain testable.

`publish_grouping.py verify <stage> --browser` binds all 253 models and
descriptors to the approved gallery evidence, validates derivative receipts,
checks map metadata and placement preservation, and rejects changed live York
inputs or references from another map to retiring assets. `install <stage>`
repeats these checks under the shared publication lock, backs up York's old
directory and map, installs the new directory/resources, and rebuilds the index.
Failures restore the old assets, map and index. Other maps remain in place.

For an installed stage, `publish_grouping.py rollback <stage>` restores the
backup only if York has not changed since installation. It rebuilds the index
so subsequent updates to other maps survive. The installer retains the approved
authoring models and creates separate optimized display copies; no web deployment
is part of this operation.

## Geometry refinement workspaces

The subsequent geometry pass is separate from the installed grouping milestone.
`prepare_geometry.py -- <asset-id>` reconciles the grounded scene's ownership
and creates immutable baseline packets under `work/york-refinement/geometry-pass-01/`.
These are diagnostic packets, not geometry approvals; source masks, state
receivers, and York lighting still need asset-specific review.

`refine_narrow_house.py` reconstructs the market southeast narrow house's absent
lower body, jetty, and roof thickness from numbered source observations. Its
native upper-body mask does not cover the lower walls, so the full-house domain
must be reviewed independently. The candidate remains private pending coverage,
complete-object and terrain-contact checks. `inspect_geometry.py -- <workspace>`
reopens a saved candidate for eight actual-material views and native isolated
and joint renders. Its supplemental framing includes the complete geometry;
it never replaces the fixed input/modified comparison cameras. Use `--output`
for a fresh inspection revision and `--crop LEFT TOP RIGHT BOTTOM` for an
explicit native source region.
