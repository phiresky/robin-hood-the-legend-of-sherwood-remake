# Map compilation evidence and history

This document retains detailed checks, local artifacts and historical measurements.
Use the [map-data checklist](map-data-checklist.md) for the concise current status.
Older counts and limitations below may be superseded by later entries.

## Wall source deformation (2026-10-04)

Export now measures the pinned models already loaded by the editor and passes
their source frames, rotated bounds and cross-section profiles to the compiler.
It keeps the original editor document and resource pins unchanged. Measurements
yield between assets/source sections so cancellation and progress can repaint.
The renderer and compiler share cross-section interpolation. Exact subdivision
endpoints prevent rounding from discarding the final collision band.

Tests compare every rendered vertex with exported physical coverage for 24
combinations of source rotation, straightening, curvature and reflection. They
also cover transformed source frames, trimming, slopes, malformed metadata,
partially invalid sources, save/reopen and asynchronous cancellation. The focused
compiler/export batch passes 178 tests; both editor typechecks and lint pass.

Wychford's isolated ridge-curtain now compiles from its actual pinned models
without calibration warnings (`work/map-compile/wychford-wall-calibration-5aVMhz`).
This found rounded navigation islands whose receiving fragments all collapse on
the integer movement grid. Those islands now produce explicit omission warnings
instead of dereferencing a missing plane. A reduced regression retains another
usable surface and rejects an entirely collapsed lift. Twelve such islands are
omitted in this wall-only audit; this is not complete wall traversal acceptance.
The diagnostic descriptor contains 3,877 sight/receiving obstacles. The audit
script's final summary initially failed on an absent optional control array after
writing the successful descriptor; that reporting bug is corrected.

The full current Wychford scene also compiles with its wall included
(`work/map-compile/wychford-wall-calibration-PNsV4t`): 21,323 sight/receiving
obstacles, two controls and two traversals. Native construction and apply/reset
of both controls pass in 22.07 seconds. The standalone test binary requires the
same `RUST_MIN_STACK=33554432` configured for Cargo tests; omitting it caused a
test-thread stack overflow before this successful run. The tower entrance and
control waypoint height mismatches listed below remain explicit omissions.

Native traversal remains a separate check. These changes do not add deformation
for wall-owned masks, lighting, sounds, material regions or stateful geometry,
and do not repair unresolved passages in placed assets.

The complete editor test batch passes with one test process at a time: 733 passed,
two skipped, no failures (`node --test --test-concurrency=1` over shared, app and
app-test files). The native Wychford receiver audit found 8,734 eligible pairs but
stopped at map coordinate `[436,1498]`: receivers 10674 and 10671 meet at the same
terrain vertex, while the audit requires a direct shared elevation edge between
them. Their heights agree at that vertex. A 67-receiver reduction reproduces the
failure in 0.10 seconds (`work/map-compile/wychford-receiver-junction-GDKJfg`).
Allowing equal-height receiver identities at the exact shared vertex moves the
failure one step further: at `[436,1497]`, the reduced case still owns receiver
48 while lookup selects 25. A synthetic four-triangle sloping fan independently
fails after crossing its center (`[50,50.600006]`, current receiver 0, queried 2).
The problem therefore includes stale receiver ownership beyond the junction,
not just boundary lookup ambiguity. These failing regressions remain under
investigation; the full crossing audit has **not passed**. No runtime receiver
behavior was changed for this diagnosis.

## Changing stair barriers (2026-10-04)

The compiler now retains asset-local movement controls on stair surfaces. Four
rotated/elevated exports and two independent copies match the native fixtures.
Native checks initially exposed a real failure: approach construction considered
every alternate barrier simultaneously, leaving an initially open stair without
actor-sized approach clearance. Shared approach construction now considers
permanent obstacles; active barriers remain enforced by runtime movement.

All three `compiled_stair_barriers` tests pass in the rebuilt native test binary,
including both travel directions, apply/reset, live route queries and independent
copies. Broader patch/state tests pass 26 cases (four ignored); the stair suite
passes five cases (three ignored), including 361 rotation angles. These were run
directly with Cargo's configured 32 MiB test-thread stack. The full game build
subsequently passed (`cargo build -p robin_rs --bin robin -j1`, 22m21s including
the shared build wait). All five `robin_level_data` approach unit tests also pass,
including permanent barriers, alternate barriers, slanted stairs and wall radii.
The complete-animation climb regression also passes: ladders, ordinary walls
and crenellated walls, 361 orientations each, both directions (2,166 routes).
It uses the shared RobinTown animation resource with a constructed map fixture;
no level records enter compilation. This validates animation-driven movement,
not rendered compositing or changing climb barriers.

Changing ladder/wall barriers remain explicit unsupported controls. Best-effort
mode retains their initial state. It also retains excess controls' initial
barriers at the sixteen-switch-per-area limit instead of aborting the export;
the eighteen-control regression checks unchanged inputs and each omission warning.

## Earlier library state compilation (2026-10-04, before wall calibration)

All ten saved library scenes compiled with the ordinary `compileMap` entry point,
best-effort mode, their pinned asset descriptors and their authored states intact.
No source-level records were read. The audit emitted descriptors only; it did not
bake images or run the resulting maps in the game.
Artifacts: `work/map-compile/current-map-states-20261004-xVEHqn/diagnostics.json`
under `level-editor`.

| Scene | Controls | Traversals |
|---|---:|---:|
| Wychford | 2 | 2 |
| croisement01 | 6 | 0 |
| croisement02 | 9 | 0 |
| croisement03 | 9 | 0 |
| derby | 5 | 12 |
| leicester | 12 | 10 |
| lincoln | 11 | 10 |
| nottingham | 10 | 12 |
| sherwood | 0 | 4 |
| york | 6 | 21 |

At this earlier audit Wychford retained three concrete omissions: the church-side-tower traversal has no
floor at its endpoint height; the keep-west-tower control has no receiving floor
at its waypoint height; and the ridge-curtain wall source lacks spline calibration.
The wall calibration omission is resolved by the later checks above; the two
placement height mismatches remain. The other nine descriptors report no
omitted state controls or traversal assemblies. All maps still carry asset review
warnings; successful compilation is not full gameplay acceptance.

The target is **new maps that play well**, assembled from reusable assets and
editor-authored terrain. Connections must follow the current placement of those
assets. Existing maps are regression examples, not a requirement to reproduce
every input record or preserve original arrangements through supplementary data.

Prioritize usable navigation, doors, sight, elevation and jumps in new layouts.
Recovered record counts below are diagnostics, not acceptance criteria. The latest
mask inventory is **2,935 unrecovered records**; earlier totals are historical snapshots.

Compilation reads **placed assets and the editor scene only**. One-time recovery
from existing levels may populate asset-local metadata; exporting never reads
those levels. Moving, rotating or duplicating an asset must carry its gameplay
with it. Global indices and connections are rebuilt after placement.

**Status:** “Working” means implemented with focused tests, not verified parity
with Derby/York. “Partial” identifies a remaining gap. “Planned” describes the
intended construction, not functionality available today.

**Unscripted map traversal:** exported maps deliberately run without a mission
script VM. Gate routes, jump selection, lockpicking validation, door hover/overlays,
patch clicks/ownership, building AI door lists, corpse door-blocking checks, lift
fall destinations, reinforcement entrances and patch animation progression now
use the loaded map domains independently of that VM. Each independent walking
lift receives its own navigation layer, while joined pieces share their lift's
layer. Receiving-plane bonds sit just inside each stair entrance on that lift's
layer. Ground-facing receiver edges are clipped only along the explicit passage,
so contour gaps or overlaps cannot cause duplicate receiver swaps. Approach
repair first follows the authored direction, then searches nearby clear positions
within 64 map units for the stock 6×3 half-diagonal. Unresolved approaches retain
their authored position with a native load warning. Binary maps are unchanged.
One-time recovery also preserves a sole static lift owner's full movement contour
instead of clipping actor clearance to its receiving footprint. Two York stair
assets have this correction published, with saved-scene hashes updated.
Actor fixtures cover both directions, four map-plane orientations, contour gaps,
overlaps and independent stairs with overlapping projections. Full-map stair
audits check receiving surfaces and height outside the passage animation and at
arrival; these use synthetic walking frames and do not certify ladder/wall
animations, every actor size or rendered sprite alignment.
Fresh published-asset diagnostics in `work/map-compile/stair-routing-HtMZ9h`
pass all 288 directed stair walks across ten maps and 6,234 sampled actor
crossings (6,178 between receiving planes and 56 between planes and ground),
using the corrected 6×3 footprint. No stair routes were skipped for permissions.
The current changes pass 4,298 engine unit tests, 72 level-data tests, 61 compiler
integration tests and 712 editor/shared tests (two skipped). Shipping codec
coverage passes 66 tests (two data-dependent tests skipped).
Earlier validation passed 4,293 engine unit tests, 68 level-data tests, 61 compiler
integration tests and 6,222 sampled actor crossings with a 6×4 half-diagonal.
Earlier client door-hover/input and 11 core overlay tests passed. The current
core overlay inventory is independent of the binary shipping codec version.

**Climb exits and landing heights:** door posture transitions now use loaded map
doors without requiring a mission VM; the old guard rejected ladder/wall exits.
Older compiled ladders and walls derive narrow approach corridors carrying each
landing's receiving plane on the lift layer. Wall corridors account for runtime
animation offsets. Crenellated transitions assign their own plane explicitly and
do not receive a duplicate corridor. The west Sherwood treehouse and York's
central-lane stone-gable house now preserve their complete asset-local movement
contours, with the corrected descriptors published and scene hashes updated.

The combined native audit in `work/map-compile/stair-routing-2IBVdk` passes
**288 stair walks, 84 ladder/wall walks and 6,234 sampled receiving crossings**.
Climb walks use Robin's complete animation rows and validate arrival, exact
sector/layer and landing receiver/height; no routes were skipped for permissions.
The report retains Lincoln's earlier `stair-routing-HtMZ9h` descriptor while that
library is being reauthored. Its current publication descriptor has no lifts and
is saved separately; this result does not certify that publication. The audit
does not certify rendered sprites, arbitrary rotated climb assemblies, every
character profile, contention between actors or mission behavior.

**Placed climb clearance:** ladder approaches and wall-bottom approaches now
receive the same collision-box clearance repair as stairs. Wall-top approaches
retain their animation-defined radius; when rounding blocks that point, the
loader searches for the nearest clear direction on that radius, without crossing
the movement boundary or an obstacle. Unrepairable approaches still warn.
A constructed ladder, ordinary wall and crenellated wall pass 2,166 full actor
routes: every whole-degree rotation plus 22.5°, both directions, using complete
Robin animation rows. A 45° wall regression previously displaced the actor past
its receiving boundary and left its height attached to the wrong surface.
Another 722 stair routes cover the same angle sweep. These checks do not certify
every sub-degree angle, asset shape, actor profile or rendered animation.

**Placed lift endpoint identities:** compilation now selects low/high doors from
their transformed 3D landing heights, using stable local door order for ties.
The native loader validates the indices and caches the matching fall and AI
destinations. Equal projected-Y endpoints are supported; rotating a lift cannot
reverse its physical low/high identities. Compiled climbs explicitly transfer
their receiving plane at sector changes, rather than relying on approach
crossings which short routes or animation teleports may skip. Older descriptors
without this metadata keep their existing endpoint/corridor behavior. New export
fixtures and updated runtime share the field; precompiled shipping containers
require regeneration for datadir v23 / mission v14.

Fresh exports in `work/map-compile/stair-routing-25heix` pass 272 stair routes,
76 ladder/wall routes and 6,196 receiving-boundary crossings with the new
endpoint metadata. All ten currently published scenes were compiled; Lincoln's
current asset revision contains no lifts and contributes no traversal coverage.
Its earlier 24 routes remain covered by the historical snapshot above. This
does not establish parity for the revised Lincoln assets.

**Ordinary route consistency:** fresh best-effort exports of all ten saved maps
with the corrected 6×3 half-diagonal pass 1,316 sampled routes in 93 seconds in
`work/map-compile/stair-routing-2IBVdk`; the Lincoln snapshot limitation above
also applies here. Historical exports using the larger 6×4 footprint
are under `work/map-compile/route-sampling-HpYhrj`. A native collision flood-fill
finds connected actor-sized samples within numbered motion sectors, then checks
forward/reverse pathfinder routes and clearance on every returned segment. The
baseline passed 1,336 routes using a 6×4 half-diagonal, taking 3,471 seconds
in this debug run. Euclidean A* with stable index ties, non-improving edge rejection
and empty-grid-cell skipping pass the same 1,336 routes in 655 seconds. That report
is saved as `route-sampling-astar-report.json`. Pathfinder corridor checks now also
stop at their first blocking line, preserving cell selection, intersection and
endpoint-inside rules. A 3,312-query comparison across two layers and three active
obstacle states matches the preceding collected-query implementation.
The final early-exit implementation passes all 1,336 routes in **76 seconds** in
the same debug test setup, versus 3,471 seconds before these optimizations.
`route-sampling-report.json` records the complete ten-map result. The same exports
also pass all 57 transition apply/reset checks and 372 directed lift callbacks;
these validate state restoration and callback membership, not complete actor
movement through every real-map traversal.
The audit writes a per-map JSON report and marks it complete only after all maps
pass. Separate transition fixtures reuse the live pathfinder with the same
footprint, checking both directions through initially closed/open barriers and
their applied/reset states. The full-map audit checks initial-state routing
consistency; it cannot prove that missing asset geometry is correct, and its
coarse samples do not cover all narrow passages, cross-sector doors, traversal
callbacks or state changes.

**Routes through changing geometry:** the same ten-map batch now passes 3,014
sampled routes across 87 independent transition states (initial, applied and reset
for all 29 movement-changing transitions). The live pathfinder is retained while
collision connectivity is sampled afresh for each state. Applying all transitions
together and resetting them in reverse order also passes 692 routes across 21
combined states. All sampled states have nonzero route coverage. These checks
cover seven maps; Leicester, Lincoln and Sherwood have no exported movement-changing
transitions in this batch. Reports are `state-route-sampling-report.json` and
`combined-state-route-sampling-report.json` beside the exports. This covers
independent and all-applied configurations, not every switch combination, actor
movement tick, door permission or narrow passage. Missing authored geometry and
unpublished state definitions remain outside this consistency check.

**Actor receiving-surface crossings:** the current ten-map batch passes 6,170
directed actor walks across 3,085 eligible receiver pairs, including 6,050 walks
in Wychford. Each tick checks receiver identity and height; an exact boundary
contact permits either receiver linked by that boundary. This audit exposed and
now covers partial edge-grid cells, nearly coincident endpoint splits and
collapsed ground slivers. `actor-receiver-crossing-report.json` records all ten
maps and the completed run. Sherwood has zero eligible pairs under this sampler's
rules. It checks one clear 24-unit perpendicular crossing per pair, on seams at
least 16 units long, with interior endpoints. Ground-only boundaries, shorter
seams, every point along an edge, state changes and real animation assets remain
outside this test's coverage.
The companion ground-boundary audit passes 52 directed walks across 26 eligible
plane-to-ground pairs in eight maps, including Sherwood. Wychford and Derby have
no eligible ground pair under these rules. `actor-ground-crossing-report.json`
records these counts separately. A regular fixture also checks entering a raised
plane and returning to uncovered ground with no receiver and zero ground height.
Together the two audits cover 6,222 directed walks, with nonzero coverage in every
map, while retaining the sampling and state limitations above.

**Exported depth PNG:** the fixture in `work/map-compile/export-depth-gpu` packages
a known two-region 16-bit depth image and a pinned sprite through the normal ZIP
exporter. The native map decoder, GPU upload and character masking pass reproduce
all 64 expected pixels at three ground depths: fully hidden, half visible and fully
visible. The offscreen GPU contract passed with this fixture. This verifies the
depth-file pipeline; it does not establish arbitrary 3D bake accuracy, full entity
mask selection, or scenery-overlay rendering. Repeat instructions are in
`docs/TESTING.md` under GPU execution.
The separate `work/map-compile/browser-depth-gpu` export comes from the editor's
real WebGL bake of ground, raised surfaces, transparent cutouts and ownership-filled
geometry with a nonzero crop origin. The browser bake contract passed, then the
native GPU test checked six locations at three character depths, including both
sides of a tile seam. This closes that fixture's geometry-to-bake-to-ZIP-to-native
pixel chain. Arbitrary scene geometry, complete entity-mask selection and rendered
scenery overlays are still not certified by it.
The `work/map-compile/browser-state-gpu` fixture extends this chain to changing
appearance: the browser packages initial/applied color and depth, the native
background loader resolves the appearance manifest, and GPU readback checks both
sides of the tile seam through initial, applied, transitioning, reset and reapplied
states. Color and character occlusion switch together in every checked state.
The browser and native GPU contracts pass. This isolates image binding/rendering
with directly assigned patch flags; it does not replace gameplay callback tests
or establish complete changing-asset coverage in the library.

**Animated scenery:** asset gameplay can now define camera-facing sprite animations
with a local anchor, sprite file/profile and center, activation/display flags and
a local 3D masking polyline. Compilation transforms the anchor and polyline with
the placed part and rebases them to export bounds. The raster center stays in
camera coordinates. Native descriptors load these into map animations, separately
from mission soldiers; older exports default to no animations. Invalid runtime
coordinates omit only the affected animation with a best-effort warning.
Sprite names accept an optional `.rhs` suffix in assets and export the basename
required by runtime resolution. A native resource test verifies Night-to-Day
fallback and selection of the authored profile and sprite center without loose
file access. This verifies resource lookup, not frame rendering.
Placement/validation and native descriptor tests pass. This is initial support,
not completed animation parity: library authoring/recovery, preview,
orientation-specific artwork and native rendering verification remain unfinished.
Animations can name a library-root-relative `resourceDirectory` ending in `.rhs.d`;
its manifest and frames must be listed in the asset descriptor's resource hashes.
Export verifies those hashes, referenced frames, profiles and sprite centers, then
packages the files under `Data/Animations/Day/<bank>.rhs.d/`. Missing or changed
resources omit the affected bank's animations with warnings; other gameplay stays
available. Resource reads report progress and honor cancellation. ZIP byte checks
and a native PNG/profile decoder check pass using the generated two-frame fixture
under `work/map-compile/scenery-resources-v2`. Its decoded profile and exported
descriptor also pass the engine's scenery-spawn and static-tick paths: the sprite
keeps its editor-authored anchor/elevation, follows the expected frame-delay
sequence, loops, and stops advancing when inactive. This does not yet verify GPU
rendering/occlusion. Export also validates action IDs, timing and geometry fields,
nonempty frame rows, contiguous unique directions, supported pixel formats and
PNG decoding/CRC within native dimension limits. Invalid pinned content is omitted
with a warning. Image checks yield between frames and preserve cancellation.
The validated `scenery-resources-v3` fixture is byte-identical to the native-tested
v2 export. Animations without an authored
resource directory still use the installed shared sprite bank.
Resource collection follows the asset/animation identities actually emitted by
compilation. Hidden or unplaced assets and individually omitted animations cannot
invalidate another placed animation's bank. Identical pinned banks in separate
asset folders share packaged files. Different pinned contents with the same bank
name receive distinct content-based export names; their compiled sprite references
are rewritten together. Pinned banks also receive separate names when sharing a
name with an installed, unpinned bank. Invalid resources omit only their bank,
preserving valid alternatives. These bindings remain editor export metadata and are not
written into the native level descriptor. Regression tests cover these cases;
`work/map-compile/scenery-placed-resources` retains byte-identical runtime files
to the native-tested animation export.
The two-bank export regression is under `work/map-compile/scenery-bank-conflicts`.
The native decoder verifies distinct pixels in both packaged banks, and the
runtime spawns both effects with separate cached profiles at their exported
positions. This checks resource identity and spawning, not rendered occlusion.
Placement also restores left-to-right vertex order when a display polyline is
reversed by rotation, preserving the same boundary shape. A 180-degree rotated
three-point fixture checks native front/behind ordering along both segments and
beyond both endpoints. Consecutive vertices that quantize to the same pixel are
collapsed. Folded and vertical projected boundaries now produce placement-specific
export warnings while retaining the effect; tests include an initially valid line
that folds after rotation. Ground effects are exempt because they render in the
background pass. Their ordering is still unsupported, not repaired by
the warning. Orientation-specific artwork and GPU occlusion remain unverified;
this is not general visual parity.

One-time animation recovery uses `pipeline/src/recover-scenery-animation.ts`.
It requires an explicit owning part and a reviewed 3D anchor whose projected
position matches the sprite/profile center. Screen-only candles stored at zero
elevation do not establish their physical attachment height; a sprite on raised
geometry would attach incorrectly if that zero were copied blindly. Recovery
converts the anchor and masking line to the owner's local frame, preserves display
flags and stores no source-record index. Tests reconstruct screen placement at a
reviewed height and check rotated, translated and raised placement. Real candle
ownership and heights remain under review; these definitions are not yet published.

Independent effects can be authored with
`pipeline/src/author-scenery-animation-asset.ts`. It accepts local animation
definitions and resource pins and returns a validated asset descriptor, an empty
GLB placement frame and an initial editor placement. Multiple effects may share
one frame. The frame has no baked artwork or collision: runtime sprite animation
is exported separately. Tests cover copying, rotation, elevation, deletion and
save/reopen without changing other geometry or the original effect. This is an
offline authoring primitive; live previews are implemented below, while published
effect assets and native rendered acceptance remain required.

Effect-only asset cards with pinned banks now show a representative static sprite
frame, chosen from the first active effect (or the first effect if all are inactive).
The thumbnail verifies resource hashes, manifest frames and the profile center,
preserves RGBA colors and removes legacy transparency/shadow keys. It lives only
on the palette canvas and cannot leak into a map bake. Ordinary model previews
remain unchanged; effects using unpinned installed banks still have no sprite
thumbnail. Unit checks and the browser fixture `tests/scenery-preview.html` pass,
including actual canvas pixels for legacy transparency and RGBA green. This does
not verify live animation, multi-effect composition or world-view placement.

The command `pipeline/src/author-scenery-animation-assets.ts` writes those assets
to a fresh library directory. A recipe contains `version: 1` and an `entries`
array; each entry supplies `id`, `name`, `map`, `origin`, local `animations`
(without `node`) and optional `resources` path/SHA-256 pins. Pinned animations
also name their `resourceDirectory`. The command validates the manifest, every
referenced frame and profile centers before creating output. It copies pinned
resources and emits descriptors, minimal model/derivative files, receipts and
`scenery-animation-assets.json` with pinned sources and initial placements.
It does not read a source level or modify the main library. From `level-editor`:

```sh
pnpm --filter pipeline exec node src/author-scenery-animation-assets.ts \
  --recipe /path/to/local-effects.json --library /path/to/pinned-resources \
  --out /path/to/new-effect-library
python3 refinement/asset_index.py /path/to/new-effect-library/3d-assets
```

The output directory must not already exist, and its parent must exist. Omit
`--library` only when all definitions use installed shared sprite banks without
resource pins. A CLI regression compiles generated placements and repackages the
copied sprite files using only the output directory. Changed hashes, missing
frames, mismatched centers and duplicate IDs fail before output is created.

Runtime library publication now includes hash-verified resources from each
animation's explicit `.rhs.d` directory, including its required pinned manifest.
Unrelated model textures and unused banks remain excluded. The staged catalog
retains the original resource pins and gameplay definitions. Publication tests
cover duplicate bank use, changed frame bytes, missing manifest pins and escaping
symlinks. The generated asset under `work/map-compile/scenery-publication-fDA5SR`
passes catalog generation and offline staging; an export rebuilt from the staged
catalog and files retains the animation and all three sprite resources without
reading original descriptors. This verifies publication transport, not preview,
live deployment or rendered scenery parity.

To repeat the animation acceptance chain, generate a fresh output directory with
`SCENERY_TEST_EXPORT_DIR` while running `app/src/scenery-resources.test.ts`.
Use that same absolute directory for the ignored native tests
`editor_exported_scenery_frames_load_with_native_profile_metadata` (`robin_assets`)
and then `exported_scenery_spawns_and_advances_frames_without_moving_its_anchor`
(`robin_engine --lib`). The first native test writes decoded profile metadata
used by the second; neither native test reads source level data.

One-time profile extraction now supports complete multi-profile PNG banks and
the editor's WebP preview atlases. From `level-editor`, run:

```sh
pnpm --filter pipeline exec node src/extract-scenery-profile.ts \
  --bank /path/to/source.rhs.d --profile 'Selected profile' \
  --out /path/to/new-effect.rhs.d
```

The new bank contains only that profile and its frames, preserving delays,
directions, offsets, centers, sound IDs and pixel-format semantics. PNG inputs
retain their exact bytes; atlas inputs are cropped without resizing. Paths,
frame bounds and native metadata are validated before output creation. Pin these
files as resources when using the standalone effect authoring command above.
The compiler requires neither the input bank nor any source level afterward.

The library's game-data atlases intentionally retain preview frames only; they
cannot provide complete animated sequences. Use a complete PNG bank for animation.
The six-frame candle profile under `work/map-compile/scenery-candle-profile-20261004.rhs.d`
was authored into a standalone asset in `scenery-candle-asset-20261004`; the native
sprite family encoder verifies all six frames. It has no level-specific placement
metadata. Offline publication and re-export from the runtime catalog preserve all
15 ZIP entries exactly. The `scenery-candle-staged-export-20261004` archive loads
three independently moved/rotated/elevated copies into the native engine without
a base datadir; cloned native sprites visit all six frames without anchor drift.
This exposed and fixed missing animation construction-order metadata, which could
prevent maps containing effects from loading. Empty and mixed control/animation
groups have regression coverage. The export uses diagnostic ground artwork;
native rendered placement and full-engine scheduling checks remain open.
The validated `authored-candle-cluster` asset is installed in the local main library
with its six frames and pinned manifest. Publication retained a rollback receipt
in `work/map-compile/scenery-candle-promotion-20261004`; a fresh export from that
main catalog also succeeds. No remote library deployment was performed.
The normal mission preload now installs referenced custom Day scenery banks as
well as characters. It selects scenery from the compiled map independently of
the mission roster and skips banks used only by other maps. This closes a separate
resource-loading gap that explicit profile injection in the earlier acceptance
test did not exercise. Alternate-ambiance authored banks remain outside this
Day-only editor export path.
The production-preload suite passes ten ordinary checks; the separately enabled
published-candle ZIP check also passes, installing six frames and their profile
with an empty character roster and no base datadir.
Native GPU acceptance now compares all six packaged candle frames against their
source pixels through the runtime sprite cache and draw path. Eighteen images
cover 1x/2x zoom and clipping at a negative origin; every pixel matches on Vulkan
and headless EGL/OpenGL. Frame dimensions vary, and opaque colors matching the
ambient shadow key retain the required one-step RGB565 adjustment. This proves
frame upload, transparency, clipping and scaling for this asset; whole-scene
ordering, fog, masks and shadow-enabled effects remain separate open checks.

**Live placed scenery:** the viewport now loads verified pinned banks for placed
asset parts, animating their initial row at 25 ticks per second with the runtime's
sentinel tick, inclusive delays and frame offsets. Placement uses the compiler's
integer coordinates. The live asset transform is used while dragging, before
the document edit is committed. Effects can be selected and dragged, contribute
to selection/framing bounds, follow copies and hidden placements, and remain
outside all bake roots. Copies share textures; map retirement disposes them.
Missing or invalid banks show a magenta marker and a warning.
Animated scenery remains visible with gameplay helpers hidden, and independently
authored assets appear in the default library selection.

The browser fixture `tests/scenery-live.html` passes actual GPU pixel projection,
legacy transparency, two-frame advancement/offsets, anchor placement, live drag,
copy/resource sharing, hiding, bake exclusion and cleanup. Unit timing checks
match the native two-frame acceptance sequence, including zero and maximum delays.
The existing palette fixture also remains separate. This covers placed asset
parts with pinned resources; background-only definitions, unpinned installed banks,
shadow previews, other action rows, orientation-specific artwork and complete
native compositing remain unfinished or unverified.

**Interior connections:** multiple entrances in an asset-local room remain
connected automatically, including after moving or duplicating the asset. Distinct
rooms in one asset retain their authored separation. The Assets inspector can
connect rooms across independent assets using map-owned `interiorConnections`.
These links use placed asset/room IDs, survive movement and save/reopen, and are
included in the editable document inside an exported ZIP. Removing a placed asset
removes its links; duplicating one asset leaves external links with the original.
Copying a compound group remaps links wholly inside that group. Asset passage
sockets continue to connect matching placements automatically. Best-effort export
warns and omits links whose rooms are hidden or lack gameplay definitions. A moved
cross-asset fixture retains both local doors and its linked third entrance in one
native room; separate-room, duplication and export round-trip tests also pass.

**Disconnected stairs and lifts:** best-effort export omits only the unavailable
traversal assembly and its walking surfaces. The owning assets retain physical
collision, ordinary landings and independent gameplay, including other valid
lifts. Joined traversal pieces are omitted together. Native regression checks
retain movement/projectile blocking and both gate links on an independent stair.
Wychford now retains 31 previously omitted church-side-tower obstacle volumes;
its independent second stair also survives (two compiled lifts instead of one).
The other nine maps compile unchanged; all ten pass native construction and
existing route probes. Native checks also pass 372 directed lift callbacks and
apply/reset all 57 exported state transitions. Diagnostics:
`work/map-compile/lift-collision-fallback`. This does not invent a missing landing
or make the disconnected stair usable.
Separated, overlapping or orientation-incompatible assembly sockets also warn
and omit affected traversal pieces during best-effort export. Every competing
piece at an ambiguous socket is omitted; export never chooses an arbitrary pair.
Strict export still reports the invalid connection.
Collapsed projected endpoints also omit only their traversal assembly during
best-effort export. The runtime requires distinct projected high/low endpoints;
collision and independent landings remain available after this omission.

**Masks on uneven terrain:** an asset mask may author a finite local
`receiverSegment` instead of requiring its anchor to match one exact elevation.
Export intersects that segment with placed receiving surfaces and requires one
unambiguous navigation layer, including surfaces covered by collision blockers.
The segment moves with the asset; mask pixels and character/projectile boundaries
remain attached to its geometry. Stacked layers, out-of-range terrain and
coplanar segments are rejected or explicitly omitted in best-effort export.
The north/south mill cottages now author an eight-unit vertical reach above and
below their anchors. This restores four Wychford mask placements (three to seven
compiled masks), with all other geometry unchanged. The nine reference maps
compile identically; all ten exports pass native construction and mask checks.
Publication backups are in `work/map-compile/mask-receiver-publication`; native
fixtures are in `work/map-compile/mask-receiver-native-v2`. This does not conform
building geometry to terrain, repair missing door endpoints or certify rendered
occlusion on arbitrary slopes. Other masks still need authored attachment rules.

**Interior entrances on uneven terrain:** interior doors may author an
`outsideReceiverSegment`. Export selects exactly one unblocked receiving surface
and moves the outside approach point onto it before rebuilding door links. The
building's midpoint, inside point, clickable contour and lock rules remain
asset-local. This option cannot be combined with a separate outside anchor or
used for lifts or ordinary passages. Out-of-range, blocked and ambiguous
attachments fail explicitly; best-effort export omits only the affected door.
The north/south mill cottages use an eight-unit vertical reach, restoring six
Wychford entrances (seven to thirteen total native doors). Native route probes
reach all six new approach points; a synthetic sloped fixture verifies walking
both directions, gate links and retained lock flags. All nine reference maps
compile unchanged, and all ten exports construct successfully. This does not
generate stairs, adjust building meshes or certify entrance animation rendering.
Publication snapshots are in `work/map-compile/door-receiver-publication`; the
native placement probes are in `work/map-compile/door-approach-native`.

**Repeatable terrain attachment authoring:**
`pipeline/src/author-terrain-attachments.ts` applies explicit local feature rules
from `refinement/catalogs/terrain-attachments.json`. Each rule names one mask,
interior entrance or physical receiver, pins its owning node and reviewed local
anchor, and declares separate upward
and downward reach. The tool refuses changed anchors, conflicting receiver
definitions, duplicate rules and unknown features. It reads only library assets,
stages rollback snapshots and republishes saved-scene descriptor pins through the
existing gameplay publisher. Running it again preserves the authored definitions.

The catalog covers twelve entrance/mask assets, including the two previously updated mill
cottages. The additional ten assets restore twelve more Wychford entrances and
ten mask tiles: **25 total native doors and 17 mask tiles** now compile. Native
character-sized approach routes pass for all twelve restored entrances. The nine
reference maps compile unchanged, and all ten exports pass native construction
and mask checks. Evidence is in `work/map-compile/ground-attachment-native`, with
publication backups in `work/map-compile/ground-attachment-publication`.
Run from `level-editor/`, using a fresh output directory:

```sh
pnpm --filter pipeline exec node src/author-terrain-attachments.ts \
  ../library ../refinement/catalogs/terrain-attachments.json \
  ../work/map-compile/terrain-attachment-review
```

Use `--apply` with a separate fresh output directory to install reviewed results.
Raised entrances and distant shared-room endpoints are deliberately not selected.
The southeast cottage's distant second entrance now belongs to the church
courtyard wall, whose geometry contains it. Leicester explicitly connects those
two asset rooms in the editor document; moving either asset carries only its own
entrance. Wychford receives the corrected asset definitions without inheriting
Leicester's map connection. The reviewed transfer preserves Leicester's 24
interior entrances, 16 rooms, door rules, navigation and collision geometry.
Publication backups are in `work/map-compile/interior-connection-publication`.
All ten saved maps reopen, compile and construct in the native engine after
publication. The other eight reference maps retain identical compiled geometry.
The courtyard wall now has a reviewed eight-unit terrain attachment around its
local doorway. Wychford's approximately 1.4-unit landing mismatch resolves without
changing Leicester's geometry or introducing a cross-building connection.

**Ordinary passages on uneven terrain:** type-0 passages can author independent
`outsideReceiverSegment` and `insideReceiverSegment` bounds. Compilation updates
their runtime approach points and receiving areas while retaining lock rules,
click polygons and transition links. Other traversal door types retain their
authored endpoints; interior destinations always retain their shared room.
Ambiguous, blocked or out-of-range receivers are rejected or explicitly omitted
during best-effort export. Cropping an inside receiver omits the passage even in
strict export. Recipes use `passage-outside` and `passage-inside` to select ends
independently.

Three stilt-shed approaches now have eight-unit attachment bounds. The narrow
ramp's destination also moves slightly inward along its existing plane to fit a
character footprint. This is asset-local authoring, applied in every placement.
Together with the courtyard entrance, these changes restore four Wychford doors
(25 → 29). Leicester changes only that ramp destination; the other eight reference
maps retain identical compiled geometry. Native diagnostics exercise both sides
of all three restored passages and the corrected Leicester landing. Publication
backups are in `work/map-compile/courtyard-door-publication` and
`work/map-compile/stilt-passage-publication`.

**Gate barriers on continuous terrain:** transition blocker surfaces can author
`terrainReach: { below, above }`. Compilation intersects the resulting local
vertical volume with receiving terrain planes, retains holes, joins triangle
fragments before grid rounding and allocates fresh state bits. Floors beyond the
finite reach remain unaffected. A `waypointReceiverSegment` binds the control to
one nearby surface, including a point inside the closed barrier; trigger contours
follow the bound control. Copies retain independent state, including after rotation.
If a terrain-bound movement/door control cannot resolve, best-effort export retains
its initial barriers and door permissions with a warning and rebuilds indices.
It does not export orphaned movement state bits. The fallback also freezes authored
visual variants in their initial appearance, keeping color/depth bake visibility
consistent with the retained barriers. A regression checks that an independently
placed, valid copy still changes appearance and keeps its movement controller.
The native fallback fixture includes a visual binding and loads without orphaned
patch references. Door-triggered controls also retain their initial state when
their anchor is unavailable: the door remains traversable with its initial rights,
and its unavailable callback is removed. A native regression passes through that
door in both directions, then verifies that another door still triggers its own
reindexed mask switch. Controls with changing sight also freeze in their initial
state: applied-state volumes export with `initial_active: false`, retaining their
receiving planes and material references. Native regression checks verify that an
inactive platform remains a height/material receiver without blocking sight or
projectiles. Existing levels without this field retain their default activity.
All ten stored maps compile to unchanged gameplay data after the door fallback
change (`work/map-compile/door-control-fallback`).
The sight fallback also leaves all ten exports unchanged
(`work/map-compile/sight-control-fallback`). Packed datadirs require version 21
and mission payloads version 12 to retain the new initial obstacle activity.

**Unavailable reveal controls:** for the supported fallback controls above, if the
anchor cannot resolve, best-effort export retains the initial masks, omits its applied masks and
rebuilds all remaining mask indices. Initial visual variants remain selected.
This also applies to fixed-height anchors, so a moved building cannot accidentally
export both mask states as active when its control loses its landing. A synthetic
fixture loads in Rust and checks that a separate mask switch still applies and
resets while the retained initial coverage stays active. Wychford's west tower
loses two incorrectly active revealed-state masks (17 → 15 exported mask tiles);
the other nine maps compile identically. Its raised control still needs a valid
walkable approach before it can operate. Diagnostic exports are in
`work/map-compile/state-control-fallback`.

The Derby south gatehouse now authors an eight-unit reach and a closed-state strip
across its opening. All three passages prohibit every actor category when closed,
so a physical barrier preserves that restriction on continuous terrain. The
compiler does not infer such barriers for doors with actor-specific permissions.
Wychford retains 29 native doors and now has a working gate transition: native
probes check all three openings closed, open and reset. Same-area passage records
still produce omission warnings, but their shared barrier remains. Derby retains
its door links and alternate permissions alongside the blocker. The other eight
saved maps compile unchanged; all ten exports pass native construction and all
57 exported transitions pass apply/reset checks. Synthetic native tests cover a
slope, an unaffected upper floor and unavailable control attachments. Gate artwork,
animation and mission activation still need separate authoring.

The pinned gameplay replacement is
`refinement/catalogs/terrain-gate-gameplay.json`; it is installed through
`pipeline/src/configure-surface-jumps.ts`, which also updates saved-scene pins.
Publication backups are in `work/map-compile/terrain-gate-publication`, and native
movement probes are in `work/map-compile/terrain-gate-probes`. These definitions
belong to the asset and follow every placement; export reads no source level.

Automatic room recovery now checks every entrance against the inferred owner's
solid doorway geometry above that entrance's landing. It reports
`interior-entrance-ownership` instead of assigning a distant entrance to the owner
of the first door. The existing 24-unit inference limit applies to every entrance;
supporting floors alone are not ownership evidence. Explicit reviewed ownership
and room partitions remain supported. Existing asset definitions are not changed
by this guard. An asset-only audit of the nine reference maps flags eleven rooms
for review, including the southeast cottage's second entrance about 1,462 units
from its owning geometry. Some other flags are near the inference threshold and
may need explicit ownership rather than splitting. The audit is recorded in
`work/map-compile/interior-locality-audit.json`.

**Physical receivers on uneven terrain:** projection receivers may also author
finite `receiverSegment` bounds. The selected unblocked terrain surface supplies
their sector/layer association; the asset's physical top plane, material and
volume remain unchanged. No extra movement boundary is introduced. The same
segment resolver handles interior approaches, with explicit rejection of blocked,
stacked, coplanar or out-of-range attachments. A native synthetic fixture checks
both terrain and asset-top heights plus character-sized routes in both directions.
The southwest edge bank is the thirteenth catalog asset. Its eight-unit reach
restores two Wychford projection bindings (2,310 to 2,312) without changing
movement geometry, doors, masks or physical shapes. The third placement is too
far above its receiver and remains unbound with a warning. Reference-map compiled
geometry remains unchanged. Staged/native evidence is in
`work/map-compile/bank-receiver-review` and `work/map-compile/bank-receiver-native`;
publication backups are in `work/map-compile/bank-receiver-publication`.

**New-layout jump connections:** assets may author an oriented jump edge plus
maximum gap, rise, drop and minimum overlap in map units. Export matches facing
parallel edges at their current placements and trims them to their shared span;
no original neighbour identity or connection coordinate is required. A rearranged
two-asset fixture checks export and native routing with character skills and
helper requirements. Native animation translation also checks takeoff toward the
destination and landing alignment at five positions along each edge, in both
directions. The edge-facing convention is the map-plane normal `(-dy, dx)`.
Existing exact sockets remain supported. Many recovered library segments are
short control lines, so migration requires usable ledges from asset surfaces;
increasing socket tolerances alone is insufficient. Automatic connections between
level ledges now check both the ordinary long-jump arc and the direct sword-fighting
flight against solid volumes in both directions, including takeoff. A low obstacle
may clear the arc while blocking the sword-fighting path, so both constrain the
exported span. Native fixtures check both styles at five positions along each
retained edge. All ten library maps compile unchanged after this additional check;
diagnostics are in `work/map-compile/sword-jump-clearance`.
Clearance also follows the runtime's fixed eight-unit airborne steps and integer
frame countdowns. Intermediate orders retain their actual endpoint rather than
snapping to each arc waypoint; the final order snaps to the landing. Both the
resulting curved flight and sword-flight overshoot are checked in addition to
the ideal paths. A thin floating-obstacle regression catches a collision between
the ideal arc and direct path, and a short-gap regression catches an overshoot
beyond the receiving edge. Native checks compare integration endpoints and test
actual per-frame movement for both styles across the exported fixtures. All ten
library maps compile unchanged; diagnostics are in
`work/map-compile/integrated-jump-clearance`.
Shoulder-assisted departures now also check the in-place 40-unit rise and the
resulting integrated flight toward the ordinary arc targets. The world Y anchor
stays fixed during this rise; it does not use the ordinary 15-unit moving takeoff.
This protects assisted jumps against overhead obstacles close to the departure
edge. Native fixtures exercise both directions and five edge positions with
upright, sword-fighting and shoulder-assisted departures. All ten library maps
still compile unchanged (`work/map-compile/assisted-jump-clearance`). These checks
do not yet certify every sprite-driven takeoff displacement.
Blocked portions are removed, clear spans are
retained and warnings explain omissions. Integer endpoints keep equal opposing
vectors and are checked again after rounding. Optional asset-authored body radius
and height add side clearance and headroom; without them only the foot path is
checked. A moved-wall fixture verifies restored overlap and native collision-free
animation. Checks conservatively include every potentially active obstacle state,
but exclude permanently inactive volumes. Volumes referenced by a surviving switch
still constrain jumps even if authored initially inactive: airborne animation does
not collision-check each frame. An unavailable control frozen with its applied
volume inactive restores the full jump span. Compiler and native trajectory tests
cover this distinction. State-dependent jump availability remains unimplemented.
Sloped ledges and
climbing-style automatic connections are omitted with warnings; explicitly authored
connections remain supported. Walkable surfaces may now declare compact `jump`
rules; the compiler derives ledges and landing bands from their placed polygons,
cuts bands around holes and concave boundaries, and can connect one ledge to several
destinations. Each retained span gets its own receiving anchor and zone. A new
two-destination courtyard verifies native routing, animation and collision without
any authored jump segments/zones. Rotated and elevated copies are also covered.
Generated takeoff lines also reserve the runtime's stock 6-by-4 half-size human
movement box. The authored inset is a minimum: compilation increases it as needed,
leaves a one-unit rounding margin and trims spans around corners and holes.
Edges without a character-sized receiving span warn and are omitted. Native
walking tests previously rejected takeoff goals that passed flight checks; the
updated courtyard, skewed-roof and complete-house fixtures now pass approach and
departure routes at five positions along every edge, including both endpoints.
The tests check goal authorization and thick movement, separately from flight.
All ten library maps retain identical geometry, with additional warnings for
unusable candidate edges (`work/map-compile/jump-walking-clearance`).
Generated connections also trim their full approach spans against the compiled
movement areas and movement-only blockers, using the runtime's goal-authorization
footprint (the stock move box minus one unit). The test covers the entire inward
landing depth, rather than only a zone anchor or sampled points. Switchable
movement blockers constrain permanent connections in both states; state-dependent
jump availability is still unsupported. A separate movable blocker fixture splits
one connection into two usable spans and restores it when moved away, without
changing sight geometry. Both retained spans pass native approach/departure checks.
A switchable version is exported as its own compiler-to-native fixture. Five
successive initial/apply/reset states verify that the excluded middle route closes
and reopens while both retained spans remain walkable at their endpoints and three
intermediate positions, in both directions through the full six-unit landing depth.
This exercises the live pathfinder and movement grid after state changes, not
only descriptor loading or equality of the generated jump records.
Both movement-blocker fixtures also pass native ordinary, sword and assisted flight
checks; their generated gates retain the character skill and helper restrictions.
The complete-house fixture remains unchanged, including its angled roof approach.
All ten saved maps still compile with identical geometry and warning counts
(`work/map-compile/jump-movement-exclusions`).
Nineteen reviewed library surfaces now carry these rules, with descriptor pins
updated in six saved scenes. All ten scenes still compile and construct native
geometry. A real rock-surface fixture checks a newly placed
rotated neighbour without saved jump records. Further library adoption remains
in progress; unmarked roof meshes do not acquire jumps. The authoring tool
`pipeline/src/configure-surface-jumps.ts` stages descriptor-bound edits, updates
scene pins and retains rollback snapshots; this publication is backed up in
`work/map-compile/surface-jump-publication`.

Surface jump rules can also author `maxLevelAdjustment`, a maximum horizontal
outer-edge endpoint displacement onto the surface's level contour before inset.
This produces constant-height takeoff lines on slightly skewed roof boundaries
without flattening the roof or changing its receiving plane. Receiving bands
are still clipped around the surface boundary and holes; collision clearance and
integer-grid rechecks still apply. Omission is unchanged unless an asset opts in.
A skewed-roof export fixture verifies that this option creates a connection that
would otherwise be omitted; native flight and gate-routing checks cover it.
A separate recovered roof-surface fixture checks a rotated copy using its solid
volume. The Bridge Square central timber house now publishes a reviewed eaves
rule with a four-unit minimum inset and landing depth, a two-unit contour adjustment limit
and sixty-unit headroom. It checks the foot path without lateral body expansion.
A complete-building fixture retains all eight solid volumes across its moved and
180-degree-rotated instances, generates a new connection, retains it after two
placement shifts and removes it when the copy moves out of range. Native tests
verify skill/helper gate rules and collision-free ordinary, sword and assisted
flight in both directions at five positions along each edge. These are traversal
checks, not a visual or whole-building parity certification; the isolated fixture
has no ground for its entrances, which are omitted with warnings.
`refinement/catalogs/roof-surface-jumps.json` records the descriptor-bound rule;
publication updates York's scene pin and retains backups in
`work/map-compile/roof-surface-publication`. Broader roof publication remains unfinished.
All ten library maps compiled unchanged after contour support and this publication
(`work/map-compile/roof-surface-native`); the walking-clearance diagnostics above
include the subsequent candidate-edge warnings.

**Reusable walkway connections:** navigation sockets now support an explicit
minimum shared span, so different widths and shifts along a shared edge can join.
Both assets must opt in; height is checked along the overlap, detached edges stay
separate, and competing overlapping neighbors are rejected. Native tests exercise
full-character routes through a partial seam. Seven Sherwood bridge/platform assets
now permit 12-unit shared spans. Their original combined geometry is unchanged;
an isolated real bridge/platform pair remains traversable after sliding the bridge
along the seam and separates after moving it away. The update is installed with
rollback data in `work/map-compile/navigation-overlap-publication`; native checks
are in `work/map-compile/navigation-overlap-native-v2`. Other exact sockets retain
their existing behavior unless explicitly opted in.

**Spline walls:** calibrated sources now export solid/opaque volumes and authored
walkable surfaces through their current spline placement. Repeats, trims, width,
flipping, slopes and corner scales follow the artwork; curved spans are subdivided
before deformation. The renderer and compiler share corner/run splitting. A
synthetic export passes native movement, routing-around-ends and ray collision
checks. Initial library definitions use conservative continuous barrier envelopes
for 19 prepared strips, with wood fences transparent to sight and masonry opaque.
These are not inferred walkable tops or detailed openings: those require explicit
asset-local surfaces/volumes. Six selectable tower sources retain their existing
physical geometry through calibrated model frames. Custom cross-section
straightening, source rotation, stateful walls and deformed doors/lifts/masks remain
unsupported and produce warnings; ordinary placed assets still support those
features. Model hashes bind calibration to the authored model. All 19 presets pass
native construction and movement probes; all ten saved maps still compile.
The publication retains rollback data under `work/map-compile/spline-gameplay-publication`.

Five curtain strips also have explicit wall-walk recipes: Derby upper/lower,
Leicester castle, Lincoln east and Nottingham castle. The authoring tool dissolves
model cap triangles into compact asset-local solids, selects the recipe's deck
height, and subtracts parapets and other higher caps from that deck. The compiler
deforms these definitions at the current placement; it never reads a level file.
Shared curve cuts, exact source endpoints and joint grid rounding keep repeated
decks connected and suppress false tiny ground islands. Straight spans avoid
unneeded subdivisions. Grounded cap solids still require manually authored volumes
for undercut openings; choosing a deck does not create stairs or tower entrances.
Native regression fixtures verify full-character routes across repeats and around
curved deck boundaries, while collision blocks parapets and ground-level crossings.
The graph-free pathfinder now considers inward area corners as well as obstacles,
with close docking candidates for narrow passages. These checks cover the tested
placements, not every bend, width or connection to another asset.
All five default-width and double-width flat wall walks pass native routing in
straight and curved placements. All 19 presets, two extra corner choices and ten
saved maps pass native construction; the saved maps retain their reviewed static
geometry. Rising straight and curved placements of all five wall walks also pass
full-character routing. Each continuous asset surface retains one navigation
region across its deformed receiving planes. Explicit asset-local deck clearances
exclude its own support caps after deformation; other placed objects still block
the deck. Subpixel deformation cracks close before movement-grid rounding without
joining physically separated decks or filling authored openings. A native fixture
checks both the route uphill and increasing receiving height. Connections to
separate stairs/towers and extreme bends or overlapping paths still need checks.
The five installed definitions have rollback data in
`work/map-compile/wall-walkway-publication`; route diagnostics are in
`work/map-compile/wall-walkway-native-v15`.
The slope-ready definitions are installed with rollback data in
`work/map-compile/wall-walkway-slopes-publication`; their native route diagnostics
are in `work/map-compile/wall-walkway-slopes-native-v3`.

Current combined drafts recover all 27 map-source movement transitions.
York's market assembly completes the movement-state ownership inventory.
Nottingham has both after assembling its four changing northern facade parts.
Croisement03 has all nine after combining its staged state assets
with the ground-receiver recovery; older ground-only batches omit two of them.
All twenty-seven recovered transitions have matching initial/applied changing-obstacle
coverage on matching movement envelopes. Nottingham's two stateful regions now
also match full walkable coverage in both states. Actor traversal remains unverified.
No map is yet published or certified at full gameplay parity.

Draft gameplay is now available in the main editor library: 1,139 structurally
validated definitions were installed, and descriptor pins in all ten saved scenes
were updated. All ten scenes reopen successfully.
This includes 17 legacy Sherwood assets whose recovered definitions were rebased
to their existing pivots. Definitions carry explicit draft issues; publication is
not parity certification. The initial publication skipped assets with changed
physical metadata or without a matching recovery. The publication tool retains
before/after snapshots and a report under `work/map-compile/main-library-draft-publication`.
A further 169 additive assets and their placements are installed in the nine
recovered map scenes: 80 lighting fields, 72 sound fields and 17 navigation or
physical assets. All existing placements, references and scene settings are
preserved; Wychford is unchanged by this addition. Models are copied into the
library with verified resource hashes, not linked to work directories. The
transaction backup is `work/map-compile/additive-gameplay-publication`. Eighteen
replacement-family assets are now installed across Croisement01/02/03, Nottingham
and York, replacing 31 old placements without overlapping old and new geometry.
All 125 replaced obstacle parts retain their world coordinates and physical flags;
723 unrelated placements and all other scene settings remain unchanged. Models
and resources are copied into the main library, and all 42 installed files pass
hash verification. The transaction backup is
`work/map-compile/family-gameplay-publication`. All ten maps compile after this
migration; full traversal and visual parity remain unverified.
Twenty previously skipped physical definitions are now explicitly reconciled in
Lincoln, Nottingham, Sherwood and York. Only obstacle volumes, collision opt-outs,
sight-join edges/caps and recovered gameplay changed; all artwork and resources
remain intact. Live and recovered model bytes match for all twenty assets, with
221 canonical part frames and their source identities verified independently.
Saved scenes retain every placement and receive only descriptor pin updates.
Default publication still rejects physical differences; this separate reviewed
operation is backed up under `work/map-compile/reviewed-physical-publication`.
These definitions remain incomplete drafts, not full parity certification.
Light bindings whose receiving geometry is still missing produce best-effort
warnings; available receivers remain active and strict mode still rejects gaps.
After family migration and physical reconciliation, all ten maps compile and
construct in Rust. Native apply/reset checks cover 56 switches, up from 47 after
the additive publication. Lighting contours pass all three ambience checks.
Nottingham now constructs 95 movement areas, 659 sight obstacles, 172 doors and
38 jump pairs; York constructs 161 areas, 1,180 sight obstacles, 254 doors and
72 jump pairs. Lincoln has 62 areas and 541 sight obstacles; Sherwood has 11
areas and 127 sight obstacles, with 15 doors and one jump pair. These counts confirm
the added definitions load; they do not establish complete gameplay parity.
Native pathfinding crosses a joined walkway seam in both directions and its
independently rotated copy, with actor clearance enforced. Routes off the walkway
and between spatially separate copies are rejected. Native roof-jump routes also
check both directions, character jump skills and destination helper requirements
on assembled and detached asset fixtures. Actor movement ticks now cross a joined
walkway and its rotated copy in both directions, using a synthetic walking
animation; receiver identity and final height match the destination plane.
Native construction derives static elevation boundaries from placed receiving
polygons, including partial contacts and transitions to uncovered ground. All ten
maps in the current diagnostic batch load with 4,871 fractional-capable boundaries
in total (3,881 in Wychford and 150 in Leicester). The same batch passes 57
transition apply/reset checks and 372 directed lift-passage callbacks.
Receiving planes remain registered when their sight obstacle is inactive; switches
control collision and navigation access rather than removing height lookup.
Boundary construction now includes these planes instead of omitting their entire
movement area. Walking actor tests verify the initial, applied and reset states,
including overlapping receiving planes with different heights; the same highest
receiver and destination height remain valid in each state. Generated boundaries
retain fractional endpoints instead of rounding them to native integer pixels.
Side probes stay within narrow overlaps and gaps, avoiding duplicate receiver
swaps across nearly coincident edges. Splitting preserves distinct float32
endpoints near vertices; collapsed ground slivers compose a direct transition
between their outer receivers. Regression tests cover distinct subpixel
seams, overlapping planes and uncovered gaps. Conflicting real receiver identities
still produce warnings. Actor ticks now follow queued, postprocessed paths around
wall ends and along curved and rising walkways in both directions; each movement
step must clear collision and match the receiving height within 0.001 map units.
Complete full-map actor traversal and real animation playback remain unverified.
Maps with dimensions that are not multiples of 64 now allocate partial edge cells
and retain their exact pixel bounds. Walking regressions cover receiving-plane
crossings inside the right and bottom strips. Wychford previously lost elevation
callbacks in its bottom 32 pixels; York also gains its partial cells. The ten-map
route audit still passes all 1,336 sampled routes after the sizing correction.
Before physical reconciliation, Sherwood's main-library browser bake passed at
1920×1088 with 115 sight obstacles and an
8,246,241-byte ZIP. That archive loads in Rust with seven door projections and
5,040 grid blocks; this differs from the more complete staged-library bake below.
Wychford's terrain adds 1,804 sloped receiving triangles derived from its authored
mesh, simplified with a one-unit error budget. Native construction passes. Water
exclusion, impassable slopes and material regions are not yet authored, so this
terrain is explicitly provisional rather than a finished traversal definition.
Its separate publication snapshot is `work/map-compile/main-library-terrain-publication`.
The combined Wychford best-effort descriptor constructs in Rust with 13 movement
areas, 2,216 sight obstacles, six doors and no jump pairs. Disconnected connections
and incomplete states are reported as omissions; this is not traversal parity.
Its complete 3600×2400 browser export now produces a 35,842,113-byte ZIP with 370
warnings. The actual archive loads color, depth, minimap and gameplay into Rust
without a base datadir. Its editor JSON preserves 46 preview actors and 14 items;
none become runtime map population. The initial generated grid had 3,691,968
blocks because joined receiving planes left unused layer slots. Compaction now
reduces that to 32,144 blocks (1,607 layers to 13), while retaining ground layer
zero, light separation and the reserved lift layer. All ten maps construct in
Rust after compaction, with exact geometry/reference equivalence after layer
renumbering. This reduces allocation; it does not restore omitted gameplay.

Editor export now requests best-effort compilation: missing definitions,
unsupported walls, unavailable door/jump connections and unbound appearance
controls produce omission warnings. An invalid lift connection omits that placed
asset's gameplay; its artwork remains. Legacy preview population is excluded from
runtime gameplay but preserved in the embedded editor JSON. Explicit Mission-tab
placements export separately as PC spawn points and NPC soldiers. Strict compiler mode remains
available for parity checks; best-effort export does not establish full parity.
Export displays phase progress and cancellation. Compilation and image/ZIP
encoding run in a worker; rendering yields between 512-pixel tiles. Browser tests
check worker responsiveness, transferred buffer ownership, cancellation cleanup
and exact synchronous/asynchronous color/depth equality. All ten saved main-library
maps pass best-effort compilation; Wychford and Sherwood's complete published-library
ZIPs were native-loaded in this publication batch.
The combined `editor-field-model-library` drafts now contain regenerated light/sound
field models and repinned scenes; older diagnostic libraries retain their old pins.
The nine source-backed `embedded-gameplay-library` drafts now embed 1,121 recovered
definitions in pinned asset descriptors. They remain incomplete local drafts.
The scene baker can now select explicit combinations of appearance patches,
independently of viewport previews, for color/depth rendering. ZIP packaging and
the Rust loader/renderer now support paired color/depth state images in disjoint
regions. Overlapping changes share complete combination tables; reset uses the
base map pixels. The editor now derives regions from potentially visible model
geometry and binds `movementTransitions[].appearances` to fresh per-placement
patch IDs. Export renders and packages every overlapping combination, with an
explicit 64-megapixel state-image budget. Dynamic shadow regions project each
controlled caster down to the lowest scene geometry using the current sun direction,
with padding for filtering and shadow bias. Independent switches can therefore
stay separate; intersecting geometry or shadow footprints still share combinations.
GPU regression checks cover low sunlight from three directions onto a lower
receiver and verify every changed color/depth pixel fits its exported region.
Rendering still evaluates full frames before cropping; tile-only state rendering
remains an optimization to implement. Automatic framing includes applied variants too.
Existing assets still need these local bindings restored. Unbound preview names,
shared aliases across different assets without a joined gameplay transition, and
manual group state overrides remain export errors. Purely visual transitions now compile when an asset
declares local appearances: the native `has_appearance` flag permits an otherwise
empty effect list, without inventing movement, sight, mask or door changes.
ZIP packaging rejects declared appearance transitions without rendered state regions,
including unresolved model bindings or geometry outside the export frame.
The shared editor/native fixture verifies apply, toggle and reset while grid flags
and door data stay unchanged. Animated mechanisms still need authored animation
resources; mission-only placeholder profiles remain excluded.
Asset-local transition `join` metadata now carries a semantic key and a point in
the transition node's frame. Equal keys with world anchors within 0.01 game units
compile to one switch when their world trigger geometry and flags agree. Motion,
sight, mask and compatible door bindings are combined; conflicting triggers,
door modes or state bindings fail explicitly. Joined placement aliases map all
member appearances to the same native patch. Moving a contact apart detaches its
switch; duplicating a complete assembly elsewhere creates an independent switch.
Compiler and native fixtures cover two joined assets, moved/duplicated placement,
combined navigation and sight apply/reset, and invalid aliases. Existing maps
still need reviewed join definitions recovered into their assets; this feature
does not resolve their outstanding shared ownership or certify map parity.
One-time recovery now preserves exported map-appearance provenance when exactly
one recovered gameplay switch belongs to the same asset. Duplicate recovery
evidence is deduplicated; conflicting placements, mission preview names, absent
definitions and cross-asset ownership remain explicit unresolved records.
The staged Derby/Leicester/Lincoln/Nottingham libraries now contain respectively
1/5/8/4 restored local appearance bindings (18 total), with 3/8/7/2 still unresolved.
Their compiled geometry is unchanged apart from the 18 `has_appearance` flags.
Pinned model inspection found all 15 direct appearance names; the other three
bindings use Leicester drawbridge endpoint variants whose applied definitions
are present. This does not verify endpoint baking. Native apply/reset still passes
for all 56 staged gameplay transitions across the nine libraries.
The baker now resolves applied model views through the same pinned primary asset
identity as gameplay compilation. Endpoint visibility and local material controls
therefore use one native switch per placement. Copies may retain the same preview
name while switching independently; sharing a name across different assets still
requires an explicit join. Model-metadata checks on all three Leicester drawbridges
confirm their initial/applied parts bind to their existing door switches and reset
exactly (3/2/2 visibility changes). A synthetic GPU bake exercises the complete
endpoint binding path, color/depth changes across a tile boundary and exact reset.
New gameplay-enabled endpoint placements now load both authored models before
insertion, register their resources together and save both pins in one placement.
Shared parts remain single instances; endpoint-only parts get the local state
visibility rule. Failed loads dispose both models and conflicting revisions fail
before publication. Drag placement preserves the authored base-height offset.
The actual-model Chromium insertion check passes for all three Leicester
drawbridges: two copies produce 6/4/4 placed parts, reopen without structural
changes, switch independently and reset exactly. The staged Leicester palette
also no longer lists its three applied views as separate base assets; all nine
staged palette indexes validate. These checks do not certify actual drawbridge
pixels, animations, traversal or a complete map ZIP round trip. Existing incomplete
placements are not automatically repaired.
The four town full-scene export gates still fail on remaining missing bindings; these
are metadata recovery results, not successful full-map bakes or parity evidence.
Croisement01 now has a complete browser-baked draft ZIP (1408×960, 4,812,420 bytes)
from its saved editor scene and pinned assets. The Rust mod scanner, archive
mount and native engine constructor load it with an empty base filesystem:
color/depth/minimap resources, embedded editor JSON, 92 sight obstacles, one mask,
16 door projections and 4,180 navigation-grid blocks pass. The archive contains
no mission actors or scripts. This is a real export/load check, not actor-traversal,
audio-playback or visual-parity certification; 102 masks remain pending.
The full-map browser runner accepts a staged library URL. Its Sherwood run found
that authored physical draft models omitted their part identifier; the generator
now retains it and the staged canopy is repaired and repinned. Import validation
remains strict. The generator also emits the unlit material required by depth
baking. Sherwood subsequently completed a 1920×1088 browser bake (7,626,337-byte
ZIP) and native loading without a base datadir: 127 sight obstacles, 15 door
projections and 9,450 navigation-grid blocks. Its 166 pending masks and unfinished
canopy appearance remain explicit gaps.
The placement regression suite also replaces a walkway or roof-jump neighbor
with an independently identified asset: geometry reconnects the new neighbor
while the displaced old one remains disconnected. Existing tests separately
cover rotated and duplicated connections; real-map actor traversal remains open.
The browser bake acceptance test now verifies an initial/applied/reset sequence
on one reused scene: color and depth change across a render-tile boundary, reset
restores every pixel, and successful/failed bakes restore borrowed materials and
scene parenting. The separate cross-language fixture checks combined patch states
and reset through editor PNG encoding, Rust decoding and CPU composition. Native
GPU coverage checks background color and sprite occlusion during state changes.
These synthetic checks do not certify existing-map visual parity. The current
renderer replaces both full textures on a state change; regional GPU updates and
efficient cropped state rendering remain unfinished. The browser fixture also
checks automatic region generation and cropped PNG values across a tile seam.

Ground-boundary compilation now supports an explicit asset-local
`preserveMovementBoundary` setting on a labelled ordinary surface. It retains
the outer contour and crossing movement obstacles separately, avoiding rounding
their implicit fractional intersections. A compiler-generated native fixture
verifies containment and thin reachability through a narrow strip, plus blocked
crossings of both boundaries. Receiving ownership and movement-state clipping
subtract crossing obstacles explicitly. Enclosed walkable islands are partitioned
into ordinary obstacles with coverage checks. This mode requires one surface per
height plane within each region; joined pieces must all opt into preservation. The offline
recovery flag `--preserve-ground-boundaries` enables draft boundary recovery;
it is not the default and is not published. The Sherwood draft now exactly
matches the reference ground walkable area after restoring its bluff as an
independent physical receiver. Its fifteen reviewed receivers match 1,023,981
sampled Rust queries. Equivalent evidence across all maps remains unfinished.

| Original map information | Construction from the editor | Status |
|---|---|---|
| Background image and minimap | Render placed models/textures; downsample the minimap. | Working |
| Character occlusion | Bake a 16-bit depth PNG from scene geometry and paired images for authored appearance states. | Static and changing browser exports pass native GPU fixtures; complete entity-mask and library coverage remain unverified |
| Projectile/view/obstacle masks and masking polylines | Rasterize asset-local coverage triangles after placement; rebuild masking boundaries, receiving layers and obstacle/state links. A depth PNG alone does **not** replace all these semantics. | Partial: explicit mask authoring, raster compilation, ZIP packaging and native state links tested; recovery/publication and visual/depth state integration remain unfinished |
| Walkable regions and layers | Transform asset-local surface polygons and heights; join coplanar regions, local multi-plane regions or matching authored boundary edges across assets, then assign fresh sectors/layers. | Partial: flat/sloped surfaces, holes and cross-asset multi-plane joins tested; join recovery/publication and full-map connectivity unfinished |
| Movement blockers | Transform explicit asset-local movement contours; optionally select permanent part/volume solids and intersect them with walkable surfaces. Sight states stay independent. | Working in synthetic tests; recovered ownership still needs review |
| Openings in movement collision | Asset-local clearances remove only the owning asset's derived collision on the matching plane; sight geometry and other assets remain intact. | Working in compiler/runtime tests; recovery geometry failures remain explicit gaps |
| Navigation graph and fast-find grid | Engine constructs routing and spatial lookup structures from compiled geometry. No copied grids or graph bytes. | Synthetic fixtures, 1,336 initial-state routes across ten maps and 3,706 routes through independent/combined switch states pass; complete connectivity and actor traversal remain unverified |
| Sight/physical obstacles | Transform asset-local shapes, per-vertex heights and solid/opaque flags. Explicit transition references select initial/applied obstacles. | Static geometry working; sight transitions verified through native initialization, apply and reset; recovered state ownership still incomplete |
| Projection surfaces / elevation | Generate height planes linked to movement areas; derive fractional receiver-crossing boundaries from all registered planes, independent of sight activation. Stair entrances connect receivers on their own lift layer. | Partial: rotated copies, curved/rising paths, sight-state fixtures, 6,234 sampled actor crossings and 288 full stair walks pass; complete full-map traversal remains unfinished |
| Doors, gates and lock rules | Transform local endpoints and optional click polygons; resolve neighbours geometrically and retain initial/alternate actor lock rules. Asset-local transition links either trigger state changes from doors or swap door permissions. | Compiler/native links implemented; recovered ownership and coverage incomplete |
| Building interiors | Asset-local rooms connect their own entrances automatically. Map-owned editor links or matching passage sockets join rooms across assets. | Compiler/native tests cover separate, moved, rotated and duplicated assemblies and editable ZIP round-trips; occupants remain mission-owned |
| Lifts / special traversal | Asset-local traversal surfaces, type, direction and endpoints; explicit local join sockets combine placed segments into one sector with multiple height planes. Independent lifts have separate layers; climb corridors transfer landing receivers. | Regression snapshots pass 288 full directed stair walks, 84 ladder/wall walks and 372 passage callbacks plus rotated/duplicated compound fixtures. Missing assemblies warn and retain independent collision. Changing lift surfaces, arbitrary rotated climb assemblies and rendered traversal remain unfinished |
| Jump zones and paired jump edges | Transform authored 3D edges or derive ledges from marked surfaces; construct receiving bands, trim flight/approach obstructions, resolve current neighbours and preserve long-jump/helper rules. | Nineteen published surfaces carry reusable generation rules; native tests cover moved buildings, multiple destinations, skills, flight, walking approaches and changing nearby blockers. Broader asset authoring and full-map traversal remain unfinished |
| Surface materials | Transform asset-local material polygons; rebuild ground, obstacle and receiving-surface links independently. Preserve receiving defaults, footprints and overlap priority. | Compiler/native tests pass; all nine imported map families have published material definitions; full geometry and receiving-material coverage remain unverified |
| Light/shadow regions | Transform asset-local planar contours, resolve ordinary or traversal receiving layers and preserve ambience filters. Conflicting anchored receivers receive separate runtime layers. | Published definitions exist for all five towns and Sherwood; unrestricted query equivalence and full placement coverage remain unverified |
| Environmental sound sources | Transform asset-local emitter polylines; retain sample IDs, timing, volume falloff, acoustic altitude, noise-covering distance and ambience filters. Global emitters need no position. | Current published-library exports include emitters in all ten saved maps, including Wychford. Native construction is verified; audible playback and complete ownership/coverage review remain outstanding |
| Animated scenery / effects | Transform asset-owned billboard anchors and masking polylines; export sprite references, pinned manifests/frames and display flags into native map animations. | Standalone authoring, offline publication, palette/live previews, ZIP-only native construction and frame playback tested; full native compositing, shadows and other action rows remain unverified |
| Interactive patches / state changes | Asset-local transitions compile initial/applied movement contours, sight-obstacle references and door links, trigger zones and fresh state bindings across affected navigation areas. | Movement, sight, door and mask switching plus paired baked color/depth states pass compiler/native fixtures; sampled switch-state routes pass in seven maps. Complete asset authoring and real-map visual coverage remain unfinished |
| Map settings | Scene identity/export bounds; terrain assets supply forest behaviour and default material. Ambience is selected by the mission. | Compiler/runtime tests pass; environment defaults are published for all nine imported map families |
| Resource banks and references | Package generated resources and resolve shared sprite/audio/profile dependencies. | Baked images and pinned scenery banks package independently; unpinned shared resources use the base installation |

The following information belongs to **missions referencing a map**, not map
assets. The minimal Mission tab stores explicit authoring in the scene's separate
`mission` field. Export includes these optional mission placements alongside the
compiled map; scenes without them remain unpopulated. Existing preview population
is never implicitly converted into runtime actors. Selecting a game-data mission
explicitly imports PC spawn slots and soldiers into editable mission entries;
other entities remain previews only. Import warnings survive saving and appear
in the export report. Across the 39 available missions, import preserves 2,463
soldiers and 218 spawn slots without dropped placement records; 204 slots retain
campaign team selection instead of inventing fixed characters. This is placement
coverage, not mission behavior parity. More complete mission authoring remains planned.

| Mission information | Intended construction | Status |
|---|---|---|
| Player starting locations | Mission-owned placements with fixed profiles or campaign-selected slots; resolve projected coordinates, navigation sector/layer and receiving surface after map compilation. Export only as `spawn_points`, including `[]` for no PCs. | Game-data mission import, sprite palette, numeric sliders and export implemented; legacy spawn fields remain read-compatible; invalid placements warn and are omitted |
| Soldiers | Mission-owned placement, facing, soldier profile and allegiance; resolve navigation and receiving surface after map compilation. | Game-data mission import remaps profiles and hostility into editable placements; all PC/soldier idle sprites are published; patrols, scripts, inventory and AI roles remain unsupported |
| Civilians, targets and rescue characters | Explicit mission placement, profiles and initial behaviour. | Planned |
| Items, bonuses and scrolls | Item assets plus placement and gameplay properties. | Planned |
| Building occupants | Actor-to-interior associations resolved after placement. | Planned |
| Patrol paths | Editor-authored waypoints with waits/actions, resolved against compiled navigation. | Planned |
| AI tactics | Authored reinforcement, ambush, seek and archery points/regions. | Planned |
| Moving carts | Moving-object assets with routes, collision and animation metadata. | Planned |
| Script points, lines and sectors | Transform named local markers/regions; generate fresh runtime references. | Planned |
| Mission scripts, objectives and triggers | Authored behaviours referencing scene instances and named asset features. | Planned |
| Mission settings | Mission-specific ambience, objectives and initial map state/door-rule selection. | Planned |

The ZIP additionally includes `editor/<map>.rhlos-map.json`, preserving unsaved
scene edits for reopening with the pinned asset library. This is editor source,
not an original game data type. **Full extracted-map gameplay parity is not yet
verified.**

The offline recovery report includes ground reconstruction area differences on
the engine's integer coordinate grid. Terrain drafts fill placed-object cutouts
and move the exclusions into asset-local movement blockers. A zero area difference
alone does not prove correct ownership; adjacent assets, remaining terrain holes,
state-dependent exclusions and elevated ground still require review.

Each recovered draft now includes a compiler-schema candidate when validation
passes. `definitionValidation` lists per-asset failures. Schema validity does not
mean all gameplay was recovered or that the assembled map compiles; drafts stay
separate from the published asset library until their missing information is resolved.

Recovery includes geometry-only assets: local collision shapes remain active
unless explicitly replaced by authored movement contours. Preview-only part bounds
do not create navigation or collision; those parts require separately authored
gameplay. Standalone
passages are assigned independently; an interior's entrances remain grouped.
`staticGeometryDiagnostic` checks a disposable copy of the current visible
geometry without state/population behaviours. Its success does not authorize
export or establish gameplay parity; `candidateCompilation` checks the full scene.

Split source surfaces are recovered using each asset part's own footprint;
overlapping or uncovered portions remain explicit ownership gaps. The latest
recovery drafts for Derby, Sherwood, Lincoln, Leicester, Nottingham, Croisement01, Croisement02 and Croisement03
pass the static base-geometry check, excluding explicitly counted movement transitions.
All eight also construct their compiled movement areas, sight obstacles and grids
in the native engine without a datadir. York passes both checks using staged
canonical tower, golden timber house and stone-shop assets, including the tower's
previously missing elevated door landing: 194 movement areas, 1,161 sight obstacles,
244 doors and 72 jump pairs construct successfully after door ownership recovery.
`export-gameplay-diagnostics.ts` generates
these explicitly labelled static probes from draft assets; the ignored
`recovered_static_exports_construct_native_geometry` test reads their manifest via
`ROBIN_ASSET_MAP_DIAGNOSTICS`. This checks construction, not movement/state fidelity.
These are unpublished drafts, not completed map exports or in-game round-trip
parity results. All nine recovered maps have now passed static construction;
no map has yet been certified at full parity, and authored maps still require
published gameplay definitions for their assets.

Ordinary walkable surfaces can declare `navigationJoins`: pairs of local 3D
endpoints on an outer edge, alongside an asset-local `navigationRegion` label.
Compilation validates each edge against its surface, transforms it with the
asset and joins only coincident, opposing boundary edges from different placements.
The assembled region retains each receiving plane and material definition.
Unmatched edges leave independent regions and produce a diagnostic; overlapping
copies, multiple matches and sockets away from the surface boundary fail.
Rotation and duplication tests preserve independent assemblies. Two separate
assets export exactly the existing multi-plane native fixture, whose reachability
check crosses the plane boundary without a door or lift. Packet conversion retains
the local edge definitions without runtime sector identities.

A staged Lincoln north-curtain pair uses this metadata on the east and west wall
assets. Its complete baseline geometry is unchanged; moving either wall one pixel
east detaches the join and increases movement areas from 113 to 114. All three
descriptors load natively with 654 sight obstacles, 89 doors and ten jump pairs.
The initial authored surface drafts and diagnostics are under
`work/map-compile/lincoln-navigation-join-native`. Repeatable migration now uses
`refinement/catalogs/lincoln-navigation-joins.json`, pinned to the source and both
asset models. Recovery validates each named surface and source owner, requires
one shared ordinary source movement region and checks the complete assembled seam
before modifying any packets. Stale pins, conflicting authoring, detached edges
and attempts to join distinct source regions fail. Source indices remain confined
to the migration recipe/report; generated asset definitions contain local edges
and local region labels only.

From `level-editor`, regenerate the current Lincoln drafts with:

```sh
node --max-old-space-size=1536 pipeline/src/recover-asset-gameplay.ts \
  --library work/map-compile/projection-material-library \
  --map work/map-compile/projection-material-library/scenes/lincoln.rhlos-map.json \
  --source library/game-data/Data/Levels/Lincoln.rhp.json \
  --mask-definitions refinement/catalogs/lincoln-masks.json \
  --navigation-definitions refinement/catalogs/lincoln-navigation-joins.json \
  --out work/map-compile/lincoln-navigation-join-recovery
```

Freshly recovered definitions reproduce the previous baseline exactly and both
independent wall moves pass native construction again. Those diagnostics are under
`work/map-compile/lincoln-reviewed-navigation-native`. The definitions remain
unpublished; recovery of the other maps' joins is unfinished. A full-edge candidate
audit found no exact Derby seam in the tested drafts; boundary/height differences
still require authoring work. This does not certify full-map connectivity or actor
traversal on recovered maps.

`pipeline/src/verify-reviewed-navigation-recovery.ts` checks the recovered packet
and compiler candidate against every reviewed edge definition, revalidates pins
and source ownership, compiles the baseline, then moves each owner independently.
It writes native descriptors for successful cases and retains placement failures;
any failure leaves `complete: false` and returns a nonzero exit status. Existing
success manifests are invalidated before inputs are read. This checks assembly
and detachment, not full source-map topology or actor traversal. For example:

```sh
node --max-old-space-size=1536 pipeline/src/verify-reviewed-navigation-recovery.ts \
  --library work/map-compile/projection-material-library \
  --map work/map-compile/projection-material-library/scenes/lincoln.rhlos-map.json \
  --source library/game-data/Data/Levels/Lincoln.rhp.json \
  --recovery work/map-compile/lincoln-navigation-join-recovery \
  --navigation-definitions refinement/catalogs/lincoln-navigation-joins.json \
  --out work/map-compile/lincoln-navigation-cli-native
```

Nottingham's `refinement/catalogs/nottingham-navigation-joins.json` supplies four
exact seams in three groups across seven assets: south gate/tower/curtain wall 2,
the sloped curtain walls 3/4, and the southwest curtain's north/south segments.
Its recovered baseline preserves all geometry and bindings after remapping sector
identities and projection-array order. Drafts are under
`work/map-compile/nottingham-navigation-join-recovery`. The baseline and six
independent one-pixel westward moves construct natively, with 114 movement areas,
741 sight obstacles, 172 doors and 38 jump pairs at baseline.

The north segment of the southwest curtain still fails movement checks in either
direction because the neighboring stair's inside endpoint loses its receiving
surface. Wall 4 also cannot move east within the existing export frame. Both
directional verifier manifests remain incomplete under
`work/map-compile/nottingham-reviewed-navigation-native` and
`work/map-compile/nottingham-reviewed-navigation-west-native`. The seven successful
westward/baseline construction cases are retained separately under
`work/map-compile/nottingham-navigation-construction-native`, with the excluded
placement failure recorded explicitly. These definitions remain unpublished.
The full-edge audit also found eleven candidate edges in York. Leicester,
Sherwood and the three crossing maps had no matches in the tested drafts. This
audit does not cover partial-edge overlaps or certify that unmatched regions
should remain disconnected.

York's `refinement/catalogs/york-navigation-joins.json` recovers three reviewed
groups across six assets: castle east round tower/great hall, cathedral precinct
terrain/stone causeway, and the west precinct footbridge/city wall. Their combined
baseline retains all geometry and bindings after sector identity remapping:
192 movement areas, 1,373 sight obstacles, 254 doors and 72 jump pairs. Drafts are
under `work/map-compile/york-reviewed-join-recovery`; the verifier output is under
`work/map-compile/york-selected-navigation-native`. Five independent one-pixel
eastward moves pass compilation. Moving the causeway fails its doorway's outside
receiving-height check (157.9875 authored versus 158.0735 on the neighboring
slope), so the placement manifest remains incomplete. The baseline and five
successful moves load natively; their construction manifest explicitly records
the excluded failure in `work/map-compile/york-navigation-construction-native`.

Seven other candidate York groups remain unreviewed. Applying all ten groups
changed movement areas from 192 to 195 and projection/sight records from 1,373 to
1,330. Isolated probes show three groups change motion topology, while four change
projection partitions and associated references. Exact edge coincidence alone
therefore does not establish complete region ownership or equivalent gameplay.
The probes and comparisons remain under `work/map-compile/york-navigation-group-probes`.
These differences need comparison against intended source topology; they are not
automatically improvements or regressions. No York join definitions are published,
and full-map connectivity/state/actor traversal remain unverified.

`pipeline/src/compare-projection-coverage.ts` compares the union of receiving
polygons on the compiler's fixed coordinate grid, grouped by exact top/bottom
planes, receiving motion area, flags and ordered material definitions. It resolves
rebuilt motion/material indices, including blocker constructor slots, and rejects
ambiguous or invalid receiving references. It preserves differences in height,
materials or coverage even when record counts happen to agree. Tests distinguish
an equivalent quad subdivision from a missing triangle and changed receiving rules.
This comparison excludes overlapping-plane priority, non-projection geometry,
state references and actor traversal.

The isolated York inner-east curtain join reduces projection records by 25 while
preserving every compared coverage group exactly. Its non-projection data also
matches after motion/interior reference remapping, and both variants load natively
with 192 movement areas, 254 doors and 72 jump pairs. The native probes are under
`work/map-compile/york-inner-wall-partition-native`. Native receiving queries now
expose a real elevation difference despite the exact coverage comparison: 51,381
half-pixel samples include 28,285 receiving points, of which 28,268 differ by up to
0.000015258789 in height. Coverage and material selection match. Comparing the
baseline with itself gives zero differences. The native loader constructs planes
from the first three polygon vertices, so a different subdivision can change
float32 arithmetic even for mathematically identical planes. The join remains
outside the reviewed catalog. This exposed the need to preserve authored receiving
planes independently of subdivision and to verify receiving priority and traversal.

Receiving materials now support ordered asset-local `planePoints`. One-time
recovery copies the three plane-defining anchors into each owned surface;
compilation transforms them with the asset and carries them unchanged through
material clipping as `projection_plane`. Native loading validates thin, planar
receivers and uses those anchors for top/bottom height evaluation. Assets without
this metadata retain polygon-derived planes. Tests cover clipping, translation,
rotation, duplicated placements, invalid anchors and native/shipping round-trips.
The binary shipping schema advances to datadir 19 / mission 10; older binary
bundles must be regenerated. Existing source files and hackable JSON without the
optional field retain their loading behavior.

York was recovered again into `work/map-compile/york-plane-anchor-recovery` and
both variants compiled into `work/map-compile/york-plane-anchor-native`. The same
51,381 native queries now have zero coverage, material or elevation differences.
This fixes the observed float32 subdivision mismatch, without accepting a height
tolerance. Exact coverage grouping still detects approximately 2.777463 square
pixels assigned to different ordered plane anchors, even though those flat planes
give identical sampled native heights. Full receiving-priority and traversal
verification is still required; the candidate remains outside the reviewed catalog
and the regenerated definitions remain unpublished.

The receiving-plane migration was also checked across all nine source maps.
`work/map-compile/all-plane-anchor-native/diagnostics.json` records nine successful
native constructions. `pipeline/src/compare-receiving-plane-anchors.ts` compares
ordered anchor triples at float32 bit precision, including the export-frame
offset, and reports receivers without anchors separately. Its baseline audit
(`anchor-roundtrip.json` in the same directory) finds all 698 explicit receiver
triples unchanged: Croisement01 27, Croisement02 21, Croisement03 14, Derby 78,
Leicester 63, Lincoln 118, Nottingham 106, Sherwood 17 and York 254. This checks
anchor values, not whether the correct source receiver owns each point.

That baseline also contained 413 elevated receivers using polygon-derived planes and default
material 0. These are generated fallback coverage outside the explicit material
supports; they need separate coverage/ownership review. Their total projected
area is not uniformly negligible: approximately 4,744.89 pixels squared in
Leicester, 3,501.57 in Sherwood, 1,331.40 in Lincoln and 735.89 in Derby. Passing
construction and anchor-value checks therefore does not establish full receiving
coverage or material parity. All recovered definitions remain unpublished.

The compiler no longer fills unsupported portions of a merged movement boundary
with default-material receivers when explicit receiving supports are present.
Implicit default coverage is restricted to the surfaces that actually author it.
This preserves openings such as the Sherwood platform hole, where the reference
receiving polygons provide no receiver. An exported synthetic platform fixture
verifies the same behavior through native queries: no receiver in the opening,
with elevation preserved on its surrounding edges.

Recompilation into `work/map-compile/receiving-gap-native` removes 412 unsupported
receivers across the nine maps. All nine descriptors construct natively; their
non-sight geometry is unchanged after remapping interior constructor references,
and all 698 anchored receivers are unchanged. That diagnostic retained one
unanchored receiver on Derby's `derby-second-drawbridge`, covering 731 square pixels.
These checks still do not certify whole-map
receiving priority, source coverage or actor traversal.

Further inspection found that last receiver was incorrectly inferred from an
editor preview bounding box, whose asset metadata explicitly says it has no sight
association. Recovery now creates no walkable surface from preview projection
placeholders. Compilation also excludes preview bounds from automatic part
collision, while explicit asset surfaces, passages and volumes remain usable.
Referencing a preview box as a gameplay obstacle requires an authored volume
instead. The bridge's actual state geometry and behavior still need asset authoring;
removing the fabricated surface does not complete that work.
The batch under `work/map-compile/preview-bounds-native` passes native construction
for all nine maps. Derby now has 60 movement areas, 348 sight records, 70 doors and
two jump pairs; the other eight compiled geometries are unchanged. No unanchored
receivers remain in this recovered static batch. This does not certify the
unrecovered state geometry or publication readiness.

`pipeline/src/inventory-patch-dependencies.ts` audits shared sight, mask and door
references across supplied patches and flags sight changes that activate receiving
projection surfaces. It keeps each patch and initial/applied role distinct, checks
for stale sight indices, and distinguishes masks by layer plus index. It does not
assign asset ownership, recover motion changes or import mission actors/scripts.

The authoring inventory in `work/map-compile/mission-map-effects.json` covers all
39 retained mission files and records hashes of each mission and its map source.
Derby's three missions all link both drawbridge patches to initial sight obstacle
267; the second additionally activates projection 268 and binds doors 37/38.
Obstacle 267 lies at the first bridge, so assigning both geometries to the second
asset would break independent placement. This shared dependency needs an explicit
map/mission ownership decision in the implementation, not an inferred asset merge.
The second bridge's visual elevation also differs between mission variants (1 vs
110), and its old preview source hash no longer matches the current JSON. No state
recipe has been approved from that stale pin.

Leicester's map patches activate projections 389, 384 and 390. Native interchange accepts projection obstacles
in initial/applied sight lists, with the same missing-reference and duplicate-control
validation as other obstacles. Runtime tests cover activation, swapping and reset:
collision follows activation, while elevation/material lookup retains all registered
receivers, including inactive ones, and navigation storage remains unchanged.
Assets can now link a walkable surface to a local part or volume with `projectionVolume`, replacing
its generated thin receiver with that volume's full geometry, thickness, flags and
material links. Existing initial/applied sight lists control its activation. Tests
cover movement, rotation, duplication, export into the native fixture, and native
top/underside collision plus opaque-ray blocking through activation and reset.
Missing links, mismatched heights, disjoint walking contours and multiple receiving
areas are rejected. Navigation can extend beyond its receiver without inventing
extra receiving coverage, matching their independent authored boundaries.
Overlapping physical/generated receivers require explicit
volumes on both surfaces, avoiding ambiguous overlap ordering.

One-time recovery now links uniquely owned, state-controlled projection surfaces
to their existing physical parts or local volumes. Across all nine source maps,
the three affected map-patch receivers are Leicester 384, 389 and 390. Fresh
Leicester recovery in `work/map-compile/projection-volume-recovery/leicester`
retains their ordered float32 vertices, top/bottom heights, physical flags and
default materials exactly. Compiled receiver indices 93, 56 and 243 respectively
bind their owning drawbridges' applied sight states. The diagnostic in
`work/map-compile/projection-volume-native` constructs successfully in Rust and
applies/resets all five recovered Leicester transitions, checking sight activation,
door rights and movement state restoration. This is not actor-traversal or visual
parity: the full scene still rejects unsupported visual states, 450 masks remain
unrecovered, and these candidates remain unpublished. Mission-carried projection
effects and shared controllers still need separate ownership and recovery work.
Physical receiving-plane validation now uses the first three ordered volume
vertices, retaining later vertex heights instead of requiring the whole volume
top to be planar. The authored walking surface must still agree with that plane;
degenerate first triples and height mismatches remain errors.

An all-map candidate audit in `work/map-compile/static-receiver-audit/audit.json`
found 557 static surface links whose uniquely owned physical parts exactly match
source float32 vertices and flags; 32 other surfaces lack that ownership/geometry
evidence. These are proposed links, not published definitions. Croisement03's 14
links compile and construct natively as 30 movement areas, 106 sight obstacles,
15 doors and 10 jump pairs. The other eight candidate maps remain rejected:
Croisement01/02, Leicester, Lincoln, Nottingham and Sherwood have physical
receivers spanning multiple generated movement areas; Derby and York first fail
on overlapping receiving-material priority. Fixing these requires navigation and
overlap authoring, not duplicating a physical obstacle across areas or flattening
its geometry. Native construction does not yet prove receiving-query, visual or
actor-traversal parity for Croisement03.

Receiver ownership now intersects authored walkable coverage with the compiled
area **including holes and blockers**. Outer-boundary overlap alone incorrectly
assigned a surrounding platform's receiver to a separate island inside its hole.
The editor/native island fixture verifies distinct receiver references, correct
stone/leaves material lookup, retained height and no direct walking route across
the gap. Physical receiving footprints themselves remain intact.

Recompilation in `work/map-compile/receiver-ownership-native` removes 96 wrongly
assigned generated receiver records from the previous static diagnostics: 2 in
Croisement01, 6 in Croisement02, 12 in Derby, 2 in Leicester, 23 in Lincoln, 1 in
Sherwood and 50 in York. Other geometry fields outside sight/building references
and warnings are unchanged; all nine diagnostics construct natively. A repeat of
the physical-part candidate audit still rejects eight maps. Derby now reaches a
real split of the east-hall receiver between two movement areas; York still first
fails material-priority checks. These remaining errors must be resolved through
navigation/overlap authoring before those candidate links can be published.

The 14 Croisement03 links now have a repeatable one-time recovery recipe in
`refinement/catalogs/croisement03-projections.json`. Run recovery with
`--projection-definitions refinement/catalogs/croisement03-projections.json`.
It validates the source/model pins, unique physical ownership, ordered float32
geometry and flags, and material references before changing any packet. Output
uses local part IDs; recipe source indices do not become runtime links. Stale pins,
changed shapes, missing material definitions and duplicate recipes fail atomically.

Fresh output in `work/map-compile/reviewed-projection-recovery/croisement03`
matches the prior audited baseline. The baseline plus independent one-pixel moves
of all 13 owning assets compile and construct natively in
`work/map-compile/reviewed-projection-native` (14 cases). Connection counts can
change when moved endpoints detach. These definitions are still unpublished:
Croisement03 retains 131 unrecovered masks, two missing movement transition
groups, and unverified visuals and actor traversal.

Croisement03's upper-terrace navigation boundary now has an explicit authoring
plane for the portion outside receiving coverage. The source-pinned recipe
`refinement/catalogs/croisement03-transition-planes.json` selects the associated
terrace receiver's plane for placing that changing contour only; it adds no
walkable or receiving surface. Recovery accepts it via `--transition-planes`;
`stage-navigation-state-assets.ts` accepts the same recipe after its ownership
argument. Missing coverage still fails when no explicit plane is supplied.

Staging created `croisement03-navigation-boundary-004` in
`work/map-compile/croisement03-transition-plane-stage-v2`. Fresh recovery into
`work/map-compile/croisement03-transition-plane-recovery` now has eight recovered
movement groups and one missing group (the multi-asset sight change). The export
in `work/map-compile/croisement03-transition-plane-native` retains exactly the
previous sight geometry, flags and material links, with regenerated area references.
All eight recovered transitions pass native apply/reset state checks. Masks,
shared state ownership, navigation coverage, visuals and actor traversal still
require verification before publication or a full-parity claim.

The remaining Croisement03 movement group now has one physical asset owner.
`refinement/catalogs/croisement03-state-assembly.json` groups complete obstacle
parts 102–105 into `croisement03-southwest-state-assembly`; one map patch enables
all four, and no other map patch controls them. The canonical staging tool retains
the complete model resources and part geometry in a common movable frame.
The staged library is `work/map-compile/croisement03-state-assembly-stage` and
fresh recovery is `work/map-compile/croisement03-state-assembly-recovery`.

All nine map movement groups now recover for this scene. The baseline and a
one-pixel eastward assembly move preserve the four parts' exact ordered float32
vertices and flags, and both pass native apply/reset checks for all nine transitions
in `work/map-compile/croisement03-state-assembly-native`. The assembly waypoint
moves with its geometry. This verifies movement/sight state binding only: patch 8
also controls layer-0 masks 122–124 (global mask records 128–130), which remain
unrecovered, along with visual states. The candidates remain unpublished and do
not yet certify actor traversal or full patch/map parity.
Mission-carried records also include traps, hiding places and
York gate effects; their presence in a mission file does not establish permanent
map ownership. The earlier recovered transition counts cover map-source recovery,
not this additional inventory. No mission population or scripts were added to maps.

The ignored native test
`recovered_projection_partitions_preserve_sampled_runtime_queries` reads a
`ROBIN_PROJECTION_COMPARISON` manifest with `before`/`after` descriptor paths and
`cases` containing `before_sector`, `after_sector`, `layer` and inclusive
`bounds: [min_x, min_y, max_x, max_y]`. It queries the runtime receiver, elevation
and material at integer and half-pixel positions, writes a sibling `.report.json`
file, and fails on any difference. This is a sampled check, not continuous-space
or actor-traversal certification. The York manifest is `projection-comparison.json`;
`projection-self-comparison.json` supplies the passing control.
The east bridge terrace candidate instead changes material bindings across
1.366211 square pixels. The riverside wall and middle outer bastion candidates
retain smaller nonzero coverage differences; no tolerance was used to accept them.
Detailed comparisons are under `work/map-compile/york-navigation-group-probes/verified-coverage-*.json`.

The native compiler interchange accepts typed mask bitmaps with character and
projectile polylines, view flags and regenerated sight-obstacle references.
Mask-state transitions reference the compiled array; loading rebuilds the native
per-layer mask references, including interleaved input layers. Mask-only transitions
can initialize, apply and reset without mission actors. Invalid type combinations,
missing layers/obstacles, malformed bitmap rows and multiply controlled masks are
rejected before loading can skip a mask and shift the references. The editor's
binary-silhouette encoder has shared fixtures checked by the native decoder,
including partial bytes, transparent rows and runs longer than one control byte
can represent. Incompressible rows exceeding the format's byte limit require
narrower bake tiles and fail explicitly. This establishes the interchange and
encoding, not full-map mask parity.

Assets can now define local coverage triangles, a receiving-surface anchor,
character/projectile masking boundaries, view flags and local obstacle IDs.
Compilation transforms this geometry, rasterizes binary coverage in 1024-pixel
tiles and regenerates front masking polylines, preserving concave vertical steps.
Character boundaries use projected coordinates; projectile boundaries use world
XY, with obstacle links supplying altitude tests. Explicit triangles preserve
cutouts and can include multiple surfaces; they are not inferred from a bounding
box or an unchanged screen bitmap. Local initial/applied mask IDs bind every
generated tile independently for each placed copy. Tests cover movement,
elevation, rotation, duplication, holes, wide-mask seams, packet conversion and
ZIP retention. An editor-generated fixture verifies native coverage, masking
rules and apply/reset behavior without source-level files or mission actors.
Existing map assets still need recovered/authored coverage and boundaries;
automatic extraction from textured meshes, visual-state resources and coordinated
depth-buffer changes remain unfinished. Assets can explicitly declare
`maskOcclusionNodes` for parts whose complete sprite occlusion is controlled by
their typed masks. Color baking retains those parts; depth baking omits only
their geometry and renders the surfaces behind them. Other parts retain their
depth contribution. This prevents static mesh depth from overriding mask
deactivation for the declared parts. It requires complete authored coverage:
the compiler does not infer this declaration from a partial mask set. Existing
assets have not yet been certified or opted in, and visual-state resources still
need integration, so this does not establish full-map mask parity.
The browser GPU test verifies identical color pixels, exposed underlying ground
depth for a declared part, and unchanged depth for an unrelated part. Unit tests
also verify declaration validation, packet conversion and visibility restoration
after a failed bake.

The Derby southwest postern is not yet eligible for mask-controlled depth. Its
other linked masks, 67/68, lack 72/98 pixels of mesh support (14/19 interior).
The coverage audit reports 11/18 separate connected repair regions, including a
33-pixel gap at `[471,2319,484,2326]` and a 32-pixel gap at
`[581,2469,591,2480]` (exclusive upper bounds). Adding the postern's collision
volume surfaces in a diagnostic probe still leaves 70/7 pixels unsupported.
Masks 70/71 remain fully supported. No depth declaration has been added to this
asset; completing two masks does not certify its other parts.

```sh
node pipeline/src/audit-mask-surfaces.ts \
  --library work/map-compile/projection-material-library \
  --map work/map-compile/projection-material-library/scenes/derby.rhlos-map.json \
  --source library/game-data/Data/Levels/Derby.rhp.json \
  --asset derby-southwest-postern --masks 67,68,70,71 \
  --out work/map-compile/postern-mask-coverage.json
```

The one-time bitmap recovery helper strictly decodes source scanlines and merges
coverage into nonoverlapping screen-space rectangles without filling cutouts.
`node --max-old-space-size=1536 pipeline/src/audit-mask-bitmaps.ts library/game-data/Data/Levels/*.rhp.json`
(from `level-editor`) verifies every pixel after reconstructing those rectangles.
All 3,027 masks across the nine source maps pass. Only 363 have obstacle links;
these are altitude-test references, not sufficient evidence of visual ownership.
The rectangles are intermediate authoring data, not asset geometry: ownership,
intersection with actual asset surfaces, local 3D coordinates, masking boundaries
and state bindings still need recovery before publication.

Surface lifting now clips this intermediate coverage against explicitly supplied
owner mesh triangles, splits overlaps where their depth order changes, and
stores only the frontmost surface in asset-local coordinates. It rejects coverage
outside the mesh instead of extrapolating height. Tests cover sloped faces,
cutouts, crossing surfaces, duplicate faces and foreground islands. This helper
is used by explicit reviewed recipes in the batch asset migration; these
synthetic tests alone do not certify existing-map mask recovery.
The mesh reader handles indexed/unindexed triangles and nested transforms in a
selected model part. Skinned/animated geometry and blended materials reject
until their state or coverage is explicitly handled. Surface clipping
uses fixed-point polygon operations; recovery then rerasterizes with the map
compiler and requires exact source pixel coverage, allowing partially covered
edge cells only when their pixel samples match. Four Derby probes (mask records
15, 27, 67 and 93) still fail coverage against their obstacle-linked candidate
assets' published meshes. They are not recovered or published as gameplay masks.
Those four candidates lack 175, 323, 72 and 332 covered pixels respectively;
the gaps include interior pixels, so accepting boundary rounding alone is
insufficient. Reviewed cottage associations also require geometry work.

Cutout (`MASK`) materials can now supply physical alpha coverage to reviewed
recovery and `audit-mask-surfaces.ts`. The reader decodes pinned asset textures,
clips mesh triangles in UV space against nearest-sampled base-level alpha, and
interpolates the original surface positions and vertex alpha. Uniform material
alpha and degenerate UV mappings are supported. Opaque provenance atlases remain
opaque; foliage's explicitly declared vertex ownership channel does not multiply
physical opacity. Tests cover holes, cutoff equality, sloping geometry, vertex
alpha, degenerate UVs and the foliage metadata contract.
UVs outside the unit square, texture transforms, linear magnification and blended
materials still reject; mipmap/minification silhouettes are not certified by this
base-level authoring geometry.

The central Sherwood oak now passes through the alpha-aware support audit:
`work/map-compile/sherwood-central-oak-alpha-support.json`. Its cutout mesh expands
to 660,698 triangles. Of seven nearby mask probes, record 150 has complete support
for all 375 pixels; records 32/34/62/70/151/153 still lack coverage. This probe does
not establish ownership or recover a complete mask definition. Subsequent asset
review places record 150 on the central platform's lower ladder, not the tree.
The platform mesh still misses 146 of its 375 pixels (24 interior pixels); records
151 and 153 also lack platform coverage. Do not recover these masks onto the
overlapping tree. The reviewed mask total remains 72. Reviewed recovery now discards mesh triangles outside the
union of the requested masks' projected bounds before expanding texture alpha.
For these seven probes it reduces the candidate geometry from 660,698 to 3,327
triangles (99.5%) with identical complete support/gap reports, recorded in
`work/map-compile/sherwood-central-oak-alpha-bounded-support.json`. Accepted texels
also merge regardless of stored alpha when vertex alpha is uniform; varying
vertex alpha retains distinct clipping thresholds. Full-tree silhouette hashes
remain identical at baseline and 45-degree rotation. This does not simplify or
alter the published tree model. A fresh Croisement03 recovery also produces
identical gameplay candidates for all 93 assets, including its recovered mask
and state geometry (`work/map-compile/croisement03-bounded-mask-recovery`).
Broader tree recovery and visual filtering
fidelity remain unfinished.

Sherwood recovery now restores three omitted physical ladder volumes (97/98/101)
into the central-oak and ladder-oak platform assets. Their authored local volumes
also supply receiving geometry, preserving thickness and ordered height planes.
Lift connections explicitly select their local traversal surface, so two ladders
can share a part frame without ambiguous bindings; volume clearance IDs are also
independent. Clearance subtraction uses fixed-point clipping for near-coincident
edges that otherwise fail to close a polygon.

`work/map-compile/sherwood-ladder-volume-recovery` validates all 81 asset drafts.
The baseline and central-platform translation construct native maps with 29 areas,
294 sight obstacles, 15 doors and one jump pair. The ladder-oak platform also
compiles independently when translated 100 units away, or one unit together with
its separate oak asset. Across all four cases the three restored volume shapes
and flags match the source at float32 precision, and all four lift endpoint sets,
directions, types and lock rules match after translation. See
`work/map-compile/sherwood-ladder-volume-native` and its generator
`work/map-compile/verify-sherwood-ladders.mjs`.

The one-unit platform-only move still fails: isolating collision for each of the
81 assets identifies the unmoved oak as the only owner whose collision removal
makes it pass. Its recovered openings lie on the original traversal planes; they
remain with the tree when the ladder moves. This is a cross-asset collision and
clearance limitation, not missing ladder metadata. Do not erase neighbouring
collision to force a successful export. Native passage callbacks pass for all
12 directed lift endpoint pairs in each of the four successful scenes (48 pairs),
with a test actor entering and leaving the expected sector and layer. These checks
do not simulate approach routing, authorization or climb animation, and do not
certify full traversal or map parity. Recovery now inventories every sight record lacking a
physical asset owner: Sherwood retains record 13 (referenced by mask 76), plus
166 unrecovered masks, one light region and five sound sources. Counts of owned
sight records establish metadata presence only, not geometric fidelity.

The same passage-callback check passes across the nine earlier static drafts in
`work/map-compile/receiver-ownership-native`: Derby 32 directed pairs, Leicester
38, Lincoln 24, Nottingham 92, York 170 and the older Sherwood draft two. The
three crossing drafts contain no recovered lifts, so they exercise no callbacks.
The updated Sherwood cases above cover its additional restored ladders.

`work/map-compile/all-sight-owner-audit/audit.json` inventories all nine source
maps against their pinned assets and current explicit ownership declarations.
Only two source sight records still have no physical asset owner: Derby 35
(referenced by mask 6) and Sherwood 13 (referenced by mask 76). Both are solid,
opaque and mouse-active and neither belongs to a state patch. The other seven
inventories have no missing owner, but that does not prove the owned geometry is
equivalent, correctly grouped, published or complete in other gameplay features.
This audit omits mask recovery and is not a publication candidate. Wychford has
no corresponding source map for this comparison.

Visual inspection identifies both missing records as separate canopies, not
non-rendering pieces of neighbouring buildings: Derby's small canvas shelter
beside the lower west curtain and Sherwood's thatched preparation-table canopy.
The reviewed `derby-obstacle-drafts.json` and `sherwood-obstacle-drafts.json`
recipes pin their source data and record that ownership. Run
`pipeline/src/author-obstacle-drafts.ts --source LEVEL_JSON --recipe RECIPE_JSON
--out NEW_DIRECTORY` to author independent assets with local physical volumes
and visible volume-preview meshes. These are explicitly unfinished appearance
drafts; they contain no mission actors, invented navigation or source-map lookup.

The staged `derby-canopy-stage` and `sherwood-canopy-stage` scenes under
`work/map-compile` reopen successfully and recover 42/82 asset definitions with
zero unowned sight records. Baseline and 100-unit canopy translations match each
restored volume's ordered vertices and flags at float32 precision and construct
native maps (`canopy-draft-native`): Derby has 60 areas, 337 sight obstacles,
70 doors and two jump pairs; Sherwood has 29/295/15/1. This does not certify the
other geometry or promote these drafts to published complete assets.

Roof-volume geometry alone still lacks 87 mask pixels for Derby record 6 and
1,311 for Sherwood record 76, including support poles and silhouette details.
The `derby-canopy-mask-audit.json` and `sherwood-canopy-mask-audit.json` reports
retain these gaps; neither mask is recovered. Textures and appearance completion
remain required. Saving/reopening also now restores an empty resource list for
scene assets whose descriptor omits that optional field, avoiding a validation
failure after compact serialization removes the redundant saved list.

Canopy drafts now accept explicitly authored visual support posts beneath the
roof. The posts are model children in the same local asset frame and add no
gameplay collision. Derby's visible front post reduces mask 6's unsupported
pixels from 87 to 32 (no interior gaps); Sherwood's three visible posts reduce
mask 76's gaps from 1,311 to 805 (381 interior pixels). These measurements are in
`derby-canopy-pole-mask-audit.json` and `sherwood-canopy-post-mask-audit.json`
under `work/map-compile`. Remaining thatch, roof-edge and timber detail gaps
still require geometry authoring; neither mask is recovered yet.
`verify-canopy-drafts.mjs --posts` verifies the complete compiled gameplay output
is unchanged for both maps at baseline and after moving each canopy 100 units.
The newer scenes are `derby-canopy-pole-stage` and `sherwood-canopy-post-stage`;
textures and completed appearances remain unfinished.

Asset character/projectile boundaries can now be explicitly open, independently
of one another; existing authored boundaries remain closed by default. This
preserves source polylines without inventing a closing edge across a concavity.
Monotone open lines retain vertical endpoint steps; other placements recompute
their front envelope. The bitmap audit also verifies all 5,166 nonempty source
polylines across the nine maps are reproduced point-for-point. This verifies the
boundary representation only, not their receiving elevation or asset ownership.

`recover-occlusion-mask.ts` combines verified coverage with explicitly supplied
boundary heights, a receiving anchor and local obstacle ownership. It emits an
asset-local definition without source layer/obstacle indices or bitmap data.
Character heights lift projected points; projectile heights preserve world XY.
Tests recompile every supported flag combination unchanged and verify movement,
elevation and independent authoring data. Missing height/ownership evidence is
rejected. The batch migration still needs reviewed inputs for existing masks;
this authoring function does not certify their recovery or state links.

Patch mask references now use the correct `{layer, index}` schema, with indices
local to each layer. Parsing rejects dangling/flat references. State-link recovery
resolves these into recovered asset-local IDs, refusing missing owners, duplicate
state IDs or implicit cross-asset coordination. Tests cover interleaved source
layers and independent links after asset duplication. All 518 state references
across the nine source maps resolve; none reuses a mask within/across patches.
This does not mean those masks have recovered coverage or published state links.

Reviewed mask migration now accepts state-controlled masks only when the whole
patch mask set belongs to one asset and one recovered local transition. It writes
the local IDs into that transition's initial/applied mask lists after geometry
recovery succeeds. Missing masks, competing controllers and cross-asset ownership
remain errors. Tests cover both phases, duplicate transition discovery and invalid
ownership; the mesh-backed migration test also exercises a controlled mask.
When a reviewed set has no movement changes or door links, recovery can create
its local mask/sight transition directly. Every referenced sight obstacle must
belong to the same asset. Unrecovered movement or door behavior is an error;
this path cannot silently replace either with a mask-only state.

Croisement03's staged southwest assembly owns all three applied masks of patch 8
(global records 128–130; layer-local records 122–124), but its mesh lacks support
for 981, 803 and 5 covered pixels respectively, including 661 and 520 interior
pixels in the first two masks. The reproducible `audit-mask-surfaces.ts` report is
`work/map-compile/croisement03-state-assembly-mask-audit.json`. These masks need
authored surface geometry and remain unrecovered; state ownership alone does not
establish mask parity. The reviewed static-mask total remains 64.

Croisement03 also has one recovered changing mask: western platform record 126,
with all 2,592 pixels supported by `croisement03-group-062`. The pinned recipe in
`refinement/catalogs/croisement03-masks.json` uses the platform's 82.00001-unit
receiving elevation and binds its initial cover to local `movement-change-5`.
Baseline and a one-unit asset move preserve exact coverage, masking rules and
transition links; baseline non-mask geometry is unchanged. Drafts are under
`work/map-compile/croisement03-controlled-mask-recovery`, and diagnostics under
`work/map-compile/croisement03-controlled-mask-native`. Both scenes pass native
apply/reset checks for all nine transitions, now including mask activation and
unchanged unrelated masks. The map still has 130 unrecovered masks. This brings
reviewed recovery at that stage to 65 masks across seven maps, including 64 static masks;
publication, receiving-layer fidelity and changing visual/depth integration remain
unfinished.

Derby's west tower now contributes two applied masks (records 200/201, with
36,837/8,192 pixels), as one local mask-only transition. Leicester's great keep
contributes initial roof mask 436 and its local sight obstacle 375 as one
mask/sight transition, retaining three local roof-obstacle mask links. Both use
their authored receiving floors for character thresholds and preserve projectile
world XY. These additions bring reviewed recovery to 68 masks: 64 static and
four changing masks across seven maps. Derby has nine reviewed masks and 227
remaining; Leicester has seventeen reviewed masks and 449 remaining.
Drafts and baseline/moved diagnostics are under
`work/map-compile/{derby,leicester}-controlled-mask-{recovery,native}`. Derby's
baseline and six independently moved assets pass exact mask and native state
checks; Leicester's baseline and eight independently moved assets do likewise.
Visual patch effects, complete receiving-layer fidelity and publication are still
unfinished; these checks do not certify full map parity.

Nottingham adds two complete prison-door mask swaps: upper prison records
365/366 (1,444/1,812 pixels) and southwest prison records 407/408 (2,422/338 pixels).
Both pairs bind to existing asset-local door-triggered sight transitions. Upper
prison character thresholds use its 250.001-unit platform; southwest thresholds
receive on ground. The updated pinned mask catalog verifies twenty source masks
at baseline and after fifteen independent asset moves, with 507 masks remaining.
Drafts and diagnostics are under `work/map-compile/nottingham-controlled-mask-recovery`
and `work/map-compile/nottingham-controlled-mask-native`.
All sixteen descriptors pass native apply/reset checks. The diagnostic additionally
passes a test actor through each mask-controlled door in both directions, checking
destination sector/layer and the triggered mask/sight changes. These are passage
callback checks, not approach routing, lock-authorisation or animation playback.
Reviewed recovery now totals 72 masks across seven maps: 64 static and eight changing.

The nine-map changing-mask support audit is recorded in
`work/map-compile/controlled-mask-support-summary.json`. It found complete mesh
support for the recovered Derby, Leicester, Nottingham and Croisement03 sets,
plus five Lincoln candidates requiring ownership review. Support from terrain
alone does not assign a building mask to that terrain. The audit is incomplete
for textured-alpha tree meshes, some terrain frame selections and non-rendering
frames; it also filters candidate names and bounding boxes. Its zero-candidate
results therefore do not establish missing geometry or absence of recoverable masks.

`pipeline/src/verify-reviewed-mask-recovery.ts` reproduces the reviewed-mask
checks from a scene, pinned library, recovery packets and source-pinned recipes.
It compares exact covered pixels, flags, both optional polylines and obstacle-link
counts, rejecting ambiguous matches rather than choosing one. A source mask
may compile into multiple bitmap tiles. Their coverage must form
an exact disjoint union with consistent layer and obstacle links, and every
compiled mask must be accounted for by a reviewed recipe. Each owning asset
then moves independently; bitmap bytes, dimensions, translated boundaries and
compiled obstacle links must remain exact. A failed run invalidates the previous
manifest, records placement errors and exits unsuccessfully. For recovered
changing masks it also checks complete initial/applied mask sets against exactly
one compiled transition, before and after movement. These are geometry/state
diagnostics, not ownership, receiving-layer, visual or full-gameplay certificates.
For example, from `level-editor`:

```sh
node --max-old-space-size=1536 pipeline/src/verify-reviewed-mask-recovery.ts \
  --library work/map-compile/projection-material-library \
  --map work/map-compile/projection-material-library/scenes/derby.rhlos-map.json \
  --source library/game-data/Data/Levels/Derby.rhp.json \
  --recovery work/map-compile/derby-hall-mask-recovery \
  --mask-definitions refinement/catalogs/derby-masks.json \
  --out work/map-compile/reviewed-mask-verification/derby
```

The six-map static batch verifies its 64 reviewed masks and 45 independent asset
moves. All 51 baseline/moved descriptors load natively. Adding the latest masks
leaves baseline non-mask geometry unchanged. Outputs are under
`work/map-compile/reviewed-mask-verification/<map>`. This broader check caught a
fractional-anchor regression on Derby's postern: mask receiver elevation now uses
the authored floating-point position, while polygon membership uses the movement
grid. A sloped fractional-anchor regression test protects this distinction.
The earlier tile-aware verifier rerun is under `work/map-compile/tile-mask-verification`;
all 61 then-reviewed source records pass and all 48 native-tested descriptors are unchanged.
The current batch also uses that verifier, including three additional Lincoln masks.
Horizontal/vertical multi-tile tests reject missing pixels, overlaps and mixed
bindings. All ten oversized source bitmaps also pass a format-only split/reassembly
check (`work/map-compile/oversized-mask-roundtrip.json`): Derby 129/168/172/173,
Leicester 24, Lincoln 268 and Nottingham 126/127/449/504. This does not recover
their asset ownership, geometry, receiving surfaces or state bindings.

Croisement01 has one reviewed static mask in
`refinement/catalogs/croisement01-masks.json`: record 25 (6,258 pixels), owned
by `croisement01-group-007`. Its projectile boundary follows scenery part 074;
character threshold heights follow the owning assembly's sloped part 007.
The receiving anchor is on adjacent navigable terrain. Coverage and both open
boundaries match exactly, including after moving the assembly one pixel east.
Both scenes load natively, and baseline non-mask geometry matches the jump-anchor
diagnostic. Drafts and native checks are under `work/map-compile/croisement01-mask-recovery`
and `work/map-compile/croisement01-mask-native`. There are 102 unrecovered masks;
complete asset coverage, mask-controlled depth and publication remain unfinished.

Seven real static masks are recovered for Derby: southwest postern records
70 and 71, with 385 and 884 covered pixels, and upper gatehouse record 105,
with 4,124 covered pixels, plus lower east/west curtain records 39/44 with
5,753/8,112 covered pixels. The curtain masks have no obstacle links; their
inner-parapet coverage is fully supported by the respective wall meshes and
their receivers use each wall's own flat 150.001-unit navigation surface.
East hall records 153/154 add 4,394/4,604 pixels of projectile-only roof-end
coverage. Their boundaries follow the hall's roof geometry, with receiving
anchors inside adjacent reconstructed ground; neither has obstacle links.
Other fully supported unlinked candidates still require ownership review;
several keep masks have support from multiple overlapping assets.
Record 105 is not referenced by any patch; the
gatehouse's separate changing masks still require state recovery. The reviewed recipe is
`refinement/catalogs/derby-masks.json`; pass it to `recover-asset-gameplay.ts` with
`--mask-definitions`. Source and model hashes pin the authoring evidence. The
migration checks source receiving layers/elevations, local obstacle ownership and
exact mesh-backed coverage, and rejects changing masks until their state recovery
is supplied. All seven definitions compile from asset data only and preserve pixel
coverage and character/projectile boundaries when their owning asset moves one pixel east.
Native construction verifies their bitmap coverage and layer registration. The
baseline and all five independently moved scenes load natively. Baseline non-mask
geometry is unchanged. Updated drafts are under
`work/map-compile/derby-hall-mask-recovery`, with native descriptors under
`work/map-compile/derby-hall-mask-native`. Derby still has 229
unrecovered masks; neither the complete asset nor map is publication-certified.

Leicester has sixteen reviewed static masks in
`refinement/catalogs/leicester-masks.json`. Projectile-only records 288 (church
side tower, 20,011 pixels) and 415 (great keep, 4,552 pixels) are joined by five
character/projectile/view masks: northeast gabled house 120 (4,433 pixels), south
stilt shed 182 (1,184 pixels), and great keep 399/402/404 (2,570/2,590/3,323 pixels).
The keep's character boundaries receive on its flat 140.001-unit surface; the
shed's boundary heights follow its own sloped surface. The house uses ground.
Nine additional unlinked masks belong to the village houses: northeast gabled
house 122/123/124 (621/265/897 pixels), northeast longhouse 132/133 (539/271),
north village cottage 141/155 (4,167/476), mill north cottage 154 (432), and mill
south cottage 166 (2,135). These receive on ground. Records 123 and 132 are
view-only and correctly export without character or projectile boundaries.
All sixteen preserve exact coverage, flags, open boundaries and local obstacle
links after each asset moves one pixel east. Native loading passes for the
baseline and all eight independently moved scenes; baseline non-mask data matches
the same-library jump-anchor diagnostic. Moving the tower detaches one jump pair
and its gate. Updated drafts are under `work/map-compile/leicester-village-mask-recovery`,
with native diagnostics in `work/map-compile/leicester-village-mask-native`.
Leicester still has 450 unrecovered
masks; these assets do not have complete mask coverage or mask-controlled depth enabled.

A broader unlinked static-mask support audit is recorded in
`work/map-compile/<map>-unlinked-mask-candidates.json`. It found 13 supported
records on Croisement01, none on Croisement02/03, 9 on Derby, 75 on Leicester,
157 on Lincoln, 44 on Nottingham, 1 on Sherwood and 140 on York. These are
candidate counts, including overlapping terrain/building support and already
recovered records; they do not establish ownership or parity. The audit excludes
patch-controlled masks and name-filtered terrain/ground/region assets. It also
records unsupported transparent meshes and missing or non-rendering frames
(6/6/4 errors on the crossings, 5 on Leicester and 16 on Sherwood). Those cases
remain unassessed, rather than being counted as evidence of absent coverage.

Nottingham's reviewed recipe (`refinement/catalogs/nottingham-masks.json`)
recovers sixteen static masks: west green shop 52/55 (2,523/1,862 pixels), upper red
house 103 (4,428 pixels), and village small hut 210 (6,932 pixels), plus eleven
unlinked records: east boarded house 21/22 (12,351/917), northeast timber house
94 (562), north dormer house 79 (1,042), south gate house 47 (32,663), southwest
wall house 75 (1,269), upper green house 109 (1,189), upper west lean-to 112
(1,494), village east cottage 138 (683), small hut 211 (5,852), and village mill
155 (4,112). These receivers are ground-level. North stone house 78 adds 948
pixels receiving on its own flat 66.957-unit landing. All coverage and boundary rules
match exactly and follow independent one-pixel asset moves. Native loading passes
for the baseline and thirteen moved scenes. Non-mask geometry matches the current
jump-anchor diagnostic; sound sources match the previous same-library mask
baseline. Four jump-zone receiving references differ from that older baseline
because of the already verified owner-anchor fix, with polygons and helper rules
unchanged. Some moved jump connections detach. Updated drafts and native checks
are under `work/map-compile/nottingham-landing-mask-recovery` and
`work/map-compile/nottingham-landing-mask-native`. There are 511 unrecovered
Nottingham masks; complete asset coverage and mask-controlled depth remain pending.

Moving the north stone house 32 pixels west also preserves its mask exactly and
loads natively (`work/map-compile/nottingham-landing-west-native`). A 32-pixel
east move exposed a neighboring mask receiver covered by movement collision.
Mask compilation now retains a uniquely identified authored receiving layer
under such exclusions, while requiring actual asset surface support and retaining
strict walkability checks for doors/jumps. The compiler change leaves the complete
baseline unchanged. The east move gets past the mask check but still fails because
the moved house's door is outside walkable ground; that placement is not certified.

Lincoln's reviewed recipe (`refinement/catalogs/lincoln-masks.json`) recovers four
masks: keep 390 (16,407 pixels), lower east curtain 192 (4,910), west south curtain
203 (4,652) and northeast square tower 207 (11,503). Their character thresholds use
the owning asset's flat receiving plane: 800.00104, 350.001, 350.001 and 415.001
units respectively, including where thresholds extend beyond navigation. Anchors
are inside both the owning recovered surface and the source receiving layer;
the split east curtain uses its own surface component. Bitmap coverage remains
entirely mesh-supported. Coverage, boundaries and obstacle links match exactly
before and after moving each owner independently one pixel east. All five scenes
load natively, and baseline non-mask geometry is unchanged. Current drafts and
native checks are under `work/map-compile/lincoln-curtain-mask-recovery` and
`work/map-compile/reviewed-mask-verification/lincoln`.
There are 424 unrecovered Lincoln masks.

Annex view-only mask 398 matches all 7,593 pixels at baseline, but remains outside
the reviewed recipe: moving its owner one pixel east makes the annex stair's lower
endpoint disagree with the neighboring slope's height by approximately 0.121 units.
The receiving slope belongs to another asset. Endpoint validation correctly
rejects the traversal connection after this placement.
The failed placement diagnostic is retained under
`work/map-compile/lincoln-raised-mask-native` with `complete: false`.

York's reviewed recipe (`refinement/catalogs/york-masks.json`) recovers twenty static
masks: scaffolded corner house 79/86 (2,777/747 pixels), southwest square corner house
201 (392), central south golden timber house 227/238 (2,126/5,856), southeast lane
eastern timber house 269 (687), south gate lane front timber house 280 (1,036),
and outer east wall stair passage 164 (7,305), plus twelve town-house records:
southwest square rear house 202/215 (421/2,097), west house 204/218 (298/515),
narrow gable house 205/207 (8,244/1,752), east timber house 213 (355), southwest
lane west jettied house 295 (685), north courtyard house 297 (820), market southwest
east timber house 325 (310), and southwest market northwest house 353/354
(2,239/1,078). Record 204 preserves its character/view rules without inventing a
projectile boundary. Mask 269 receives at ground level;
164 uses its owning passage's flat 160.001-unit plane, including the character
threshold beyond navigation. Mask 86 has no character threshold and receives on
the owning house's flat 152.001-unit platform. The remaining receivers use the
flat 90.00101-unit town surface. All twenty match in the baseline and after independent one-pixel
asset moves. Native loading passes for the baseline and all fourteen moved scenes.
Adding the town-house masks leaves baseline non-mask geometry unchanged. Updated drafts and
native checks are under `work/map-compile/york-town-mask-recovery` and
`work/map-compile/york-town-mask-native`. York still has 808 unrecovered masks.
These assets have incomplete mask coverage and do not enable mask-controlled
depth. None of these maps is certified for complete gameplay or publication.

The scaffolded-house movement check exposed a landing anchor selected from a
neighboring asset's portion of a shared jump zone. Recovery now intersects the
unblocked landing region with the owning asset's receiving footprints before
choosing an anchor. It selects a point on the integer movement grid before
evaluating elevation, avoiding fractional-point/rounded-point slope mismatches.
Zone polygons, jump edges and helper rules are preserved. York's baseline now
uses a corrected receiving-sector reference for one zone; all other compiled
fields remain unchanged. Moving the scaffolded house detaches two jump pairs
and their gates without invalidating its neighbor's remaining landing anchor.
All nine maps retain all 173 recovered pairs, pass static compilation, and load
natively in the jump-anchor regression batch. Updated York recovery is under
`work/map-compile/jump-anchor-recovery/york`, with baseline/moved native checks
under `work/map-compile/york-jump-anchor-native`. Traversal fidelity and complete
map publication remain separate requirements.

Recovery discards faces outside a mask's bounds before fitting their depth
planes. This avoids numerical failures from unrelated nearly edge-on faces
without relaxing planarity or coverage checks for contributing surfaces.

`pipeline/src/audit-mask-surfaces.ts` checks pixel support for an explicitly
selected asset and mask indices. It pins source/model/scene hashes, reports missing
pixel counts and repair bounds, and checks whether every mask in each affected
state set was selected and supported. This is geometry evidence only, not ownership
or gameplay certification. Recovery now checks this support before expensive
surface clipping and reports interior gaps separately from silhouette edges.

The Derby upper gatehouse state set (patch 3, records 217–229) is not recoverable
from its current mesh: 217/218/219/221/223/224/225/226 lack respectively
362/960/219/565/121/70/65/206 covered pixels. Every failing record includes interior
gaps. Records 220/222/227/228/229 have full pixel support, but this does not justify
publishing a partial state set. Repair the asset geometry or add reviewed local
occlusion surfaces before recovering that state. The reproducible audit is:

```sh
node pipeline/src/audit-mask-surfaces.ts \
  --library work/map-compile/projection-material-library \
  --map work/map-compile/projection-material-library/scenes/derby.rhlos-map.json \
  --source library/game-data/Data/Levels/Derby.rhp.json \
  --asset derby-upper-gatehouse \
  --masks 217,218,219,220,221,222,223,224,225,226,227,228,229 \
  --out work/map-compile/gatehouse-mask-state-audit.json
```

A separate probe adding the same asset's existing obstacle-volume faces closes
record 225's pixel gap, but seven other records still have missing interior
coverage. Those volumes therefore cannot complete the state set either; no
supplemental surfaces or partial state bindings have been published.

Material recovery stores ground regions on terrain, obstacle regions on their
owning parts, and receiving defaults/region references on asset-local surfaces.
Receiving footprints retain material across blocked portions omitted from walking
contours. Material boundaries split projection faces without splitting navigation;
bounding-height priority and authored tie precedence resolve overlapping receivers.
An empty ground-material list correctly activates no ground regions; obstacle-only
regions do not become ground water or footstep materials. Native tests verify
raised material overrides and defaults, independent ground lookup and traversal
across material boundaries. Rotated/duplicated asset tests rebuild local references.
All nine recovered drafts compile and construct native geometry with these definitions;
all 397 non-lift doors, room memberships and door-linked bindings still match.
A 16-pixel sampling probe found matching material codes at 34,120 points shared
by source and compiled receiving surfaces, plus 88 compiled samples with no matching
source receiver. This is a sampled material check, not complete geometry or gameplay
parity. Definitions remain unpublished and receiving-geometry gaps remain open.

Terrain drafts also carry forest behaviour and fallback material. Recovery
normalizes clearance crossings introduced by integer rounding, preserving valid
regions instead of discarding a polygon whose signed area cancels. Nottingham's
hidden prison part retains its gameplay frame and passes the static check.

Sound recovery attaches global emitters to terrain and local emitters only when
their complete geometry has one containing asset. Overlapping parts within that
asset use a stable local frame; containment spanning different assets remains
ambiguous. Ambiguous/unowned sources
remain explicit gaps; they are not silently attached to terrain. Shared audio
samples are referenced from the base installation, rather than bundled in the ZIP.

Leicester's west moat tower owns source records 12/13 and its southeast cottage
owns record 15 despite overlapping part footprints. All three compile exactly;
moving either asset independently by one pixel preserves the corresponding
emitter displacement and all acoustic parameters. Native construction passes for
the baseline and both moved scenes (85 areas, 503 sight obstacles, 105 doors,
23 jump pairs). Baseline non-sound geometry is unchanged. This staged recovery
accounts for 10 of 24 Leicester sound sources; 14 remain unresolved.

Reviewed environmental lines can also be authored as independent sound-region
assets with a non-rendering gameplay frame. This is an explicit asset-authoring
step, not an automatic fallback for unowned emitters. Derby's west and north edge
emitters (source records 4/5) are authored this way by
`refinement/catalogs/derby-ambient-sounds.json`. The authoring command checks the
source hash and writes standalone descriptors, empty frame models, identical
runtime derivatives with hash receipts, and pinned placement references;
compilation reads only those assets. Catalog publication preserves the
non-rendering gameplay-frame marker and acoustic definitions.

```sh
node pipeline/src/author-ambient-sound-assets.ts \
  --source library/game-data/Data/Levels/Derby.rhp.json \
  --recipe refinement/catalogs/derby-ambient-sounds.json \
  --map Derby --out work/map-compile/ambient-authoring-library
```

The staged scene `ambient-authoring-library/derby-ambient.rhlos-map.json` reopens
with the two new assets. Both sound definitions compile exactly, including
polylines, delays, attenuation, altitude and ambience. Moving the west zone 50
pixels east changes only its emitter geometry. Native construction checks sample
selection, preserved source handles, polylines and delay parameters before/after
the move. This composed Derby diagnostic accounts for 5 of 12 sound sources;
seven remain unresolved. Assets and the composed diagnostic remain staged, not
published as a complete map. Recovery recognizes the placed sound-region assets,
matches their compiled definitions to exactly one unclaimed source each, and
preserves their asset-local definitions. Duplicate or mismatched sources fail
instead of being counted twice. The integrated Derby report contains 43 assets
and seven pending sound sources.

Additional source-pinned ambient recipes cover Croisement03's north edge,
Leicester's northwest edge, and Nottingham's north and northwest edges:
`croisement03-ambient-sounds.json`, `leicester-ambient-sounds.json`, and
`nottingham-ambient-sounds.json` under `refinement/catalogs/`. These four
air-altitude environmental lines have standalone local frames. Their authored
scenes reopen with pinned descriptors and retain the input scenes' state metadata.
Static diagnostic exports preserve every source field and pass native loading
before and after moving each region independently by 50 pixels. Non-sound
compiled geometry stays identical for each move. All four asset definitions and
runtime derivatives also pass offline publication staging.

The staged boundary-sound recovery accounts for 1/6 Croisement03, 11/24 Leicester,
and 6/24 Nottingham emitters. Five, thirteen and eighteen respectively remain
unresolved. These checks do not establish complete map parity or publication:
changing geometry, masks, lighting and remaining emitter ownership still have
separate outstanding requirements.

Light recovery preserves projection priority and fits receiving planes from the
leading three vertices. Elevated light contours may extend outside navigation
when their intersecting receivers agree on one plane, including raised terrain
on layer zero. A non-walkable receiving-footprint notch does not establish a
ground plane; uncovered potentially walkable portions still require a valid
plane, including areas opened by state changes. Ownership can span several parts
of one asset, but their combined footprints must cover the entire light polygon,
including its interior; enclosed gaps and competing asset owners remain errors.
The previous all-map pass recovered 35 of 149 light/shadow regions into asset-local
drafts, including six additional regions on Leicester's keep, west wing and moat
towers. All 35 exported contours and ambience masks match source records, and
the previously recovered regions remain covered. Light regions
also resolve onto stair/lift traversal surfaces; a native test verifies ambience
filtering on the traversal layer without affecting the ground layer or door links.
All nine static diagnostics constructed successfully. The other 114 regions then needed
receiving-geometry fixes, ownership review or multi-plane authoring; they are not
silently assigned to terrain. These drafts remain unpublished.

Splitting multi-plane contours introduced rounding errors in all eleven current
candidates, including loadable York 12/14 descriptors. Recovery now preserves the
complete integer contour and records asset-local receiving anchors independently
of its reference plane. Compilation copies that contour to each resolved layer,
deduplicating repeated anchors on the same layer. Surface partitions establish
ownership and locate anchors; their fractional cut vertices are not exported.
The strict piecewise recovery helper still rejects contour changes after rounding.

York regions 12/14 now recover to the west-town terrain asset with exact original
contours on layers 30/68 and 30/92 respectively. This raises the recovery evidence
to 37/149 regions; 112 remain pending. The native diagnostic checks contour
registration and activation for ambience masks 1, 2 and 4. Updated Leicester,
Lincoln and York drafts are in `work/map-compile/receiver-light-recovery`, with
descriptors in `work/map-compile/receiver-light-native`. Independent receiver
movement and duplicate-layer handling pass compiler tests. Moving the large York
terrain asset alone by one pixel merges a bridge passage's two areas. Unrestricted,
non-clickable passages can now carry `allowContinuous` in their asset definition:
they remain ordinary doors while their areas are distinct and are omitted with a
diagnostic when both endpoints share one area. Recovery sets this flag only when
both sets of access rules are unrestricted and no patch refers to the door.
Interactive, restricted and state-controlled doors cannot be omitted this way;
remaining door/state indices are rebuilt after omission. York's original-placement
output remains identical, including all door references.

The independent terrain move now passes that bridge check but fails another
passage's receiving-height check: its outside endpoint is at height 46.2952 while
the receiving slope at the moved position is 47.8476. This is retained as an error;
independent full-scene terrain movement is still not verified.
Translating the entire York scene one pixel east preserves both regions' exact
contours and ambience on both receiving layers; that check preserves existing
connections and does not replace the independent terrain-movement check.

Jump recovery produces asset-local drafts for all 173 pairs across nine maps.
`compare-jump-geometry.ts` verifies exact endpoint coordinates, polygon boundaries,
helper flags and long-jump flags against the reference records, allowing rebuilt
indices, reversed polygon winding and reordered pairs. All 173 pass. Native checks
also verify paired lines, endpoint elevations and registration on landing sectors.
These checks do not establish traversal connectivity or full gameplay parity.
Edge elevations remain independent of fractional
surface heights. Extraction now preserves the third endpoint coordinate and
remaps zone references, retaining both destinations when a crop crosses a pair.
The passing static diagnostics also include their recovered jump definitions.
Croisement03's door topology now survives recovery: asset-local navigation-region
labels preserve separate coplanar areas, including when their boundaries touch.
Labels are scoped to each placement; unlabelled surfaces retain normal merging.
One local region can now span multiple height planes while compiling to one
ordinary movement area. Each projection plane retains its height, and a native
engine fixture verifies walking across the shared boundary without a gate or lift.
Recovery preserves this relationship when all supports have one unambiguous asset
owner. Ordinary regions spanning different assets still need explicit join authoring.
Ground recovery reports per-region differences as well as overall coverage.
When two pieces have one shared straight cut and lie on opposite sides, recovery
uses that cut to divide the navigation surface without trimming its outer boundary
to the mesh. This preserves Nottingham's stair landing. Missing or ambiguous cuts
keep their unresolved footprint gaps; no nearest-owner assignment fills them.
Hidden mesh parts retain coordinate frames for explicit gameplay; whole hidden
placements remain excluded. Hidden sight geometry is included only when explicitly
referenced by a transition. Nottingham now passes the prison-frame and castle-door
checks and its elevated stair landing. York's compound lift now has
asset-local segment connections, and its staged canonical tower restores the
interior-door landing surface. Lift recovery matches shared
edges once and stores local sockets, never runtime references between assets.

Stable terrain is recovered even when its movement area has changing obstacles.
The recovery inventory preserves all 27 changing-obstacle groups, their initial
and applied contours, and patch associations. Twenty-one now recover into asset-local
movement/sight transitions: five in Croisement01, eight in Croisement02, seven in
Croisement03 and one in Nottingham. Seven belong to physical assets;
fourteen navigation-only boundaries have newly staged assets and editor placements.
These non-rendering assets carry their own local contours and support independent
movement and duplication. They add no mission actors or scripts. Their models,
descriptor hashes and editor index entries are staged but not yet published.
Each transition has one unambiguous asset owner and explicit stable movement
contours or a local list of permanent collision solids. The latter keeps a mixed
asset's unchanged parts and their clearances without deriving permanent collision
from the changing endpoints. Croisement03's compound obstacle uses three permanent
parts and one changing part. Two additional pitched-cover assets combine paired
sloped volumes that share one ridge and cover mask; both pass real-browser
loading, insertion, save/reopen and rendering checks. Their visual/mask state
export remains pending.
Changing contours are split across receiving planes while preserving holes and
projection priority. Output stores local geometry and references, with fresh
movement bindings allocated during compilation. The other six still need explicit
ownership or elevated receiving geometry. One remaining
navigation-only contour extends beyond its elevated receiving surface and is
rejected rather than assigned an inferred height. `movementTransitionRecovery` records
the recovered groups, and each unresolved group has a specific failure reason.
Visual states and effects remain separate pending work.
Door links use local endpoint IDs; compilation allocates fresh non-lift door indices
for each placement, including duplicates. Native fixture tests verify both link
directions, permission changes and restoration on reset. Door-only transitions
are supported without adding navigation or sight changes. Offline recovery maps
source door indices into local endpoint IDs and reports missing owners, cross-asset
links and unrecovered geometry in `pending.doorTransitionBindings`. All 28
door-linked patches across the nine extracted maps now recover: seven in
Nottingham, nine in Lincoln, five in York, five in Leicester, one in Croisement03 and one in Derby.
Explicit `door_sources` authoring declarations cover gate passages whose empty
openings lie beyond nearby wall geometry. They require unique source door indices,
a rationale and one pinned asset frame, and reject conflicting state ownership.
The compiler receives only local endpoints. Derby's two gatehouse declarations
restore its remaining five doors: all 42 non-lift doors and the three-door permission
transition now match source geometry and rules. `declaredDoorOwnershipRecovery`
records the one-time mapping.
Nottingham declarations restore four rooms and five doors in the two green market
frontages, north dormer house and castle main hall. Further declarations attach
parallel passage lanes to the north gate gallery, stream wall and south gate arch.
The castle hall and watchtower form one 35-part asset, keeping their shared
three-door interior together when moved. Nottingham now matches all 100 non-lift
doors, 45 interiors and seven door-linked patches in the geometry/rules comparison.
The staged combined asset passes editor insertion, rendering and save/reopen checks.
Static merging preserves component annotations, translates declared bounds and
namespaces appearance bindings without changing their resolved behavior. Its GLB
writer retains near-identity transforms so binary round trips meet the existing
world-transform tolerance. These assets and gameplay definitions remain staged.
Sherwood's two camp-hut declarations bind entrances to their wall frames within
the current grouped hut assets, including their roofs. Two treehouse declarations
keep the central-west and west rooms with their huts rather than the overlapping
oak/platform assets. All five non-lift doors and five shared interiors now
match the reference geometry/rules. Missing ladder ownership and other navigation
gaps remain separate from this door comparison.
Lincoln declarations attach the hall-terrace gate and western-tower passage to
their corresponding revealed assets, the shed entrance to its room, and all three
keep-floor entrances to one shared keep interior. These restore six doors and two
permission transitions without assigning rooms to supporting terrain. Two further
passage endpoints lie on physical supports outside their assigned receiving areas.
Explicit extraction declarations identify the containing support and an anchor in
the linked area. Recovery checks both, then retains only local heights/coordinates.
The sloped wall-walk step owns one passage; the annex owns the other gate and its
transition. Lincoln now matches all 59 non-lift doors, 19 interiors and nine
door-linked patches in the geometry/rules comparison.
Assets now support optional `outsideAnchor`/`insideAnchor` door coordinates for
selecting receiving areas independently of the visible/traversal endpoints.
These local anchors move, rotate and duplicate with the asset; no sector indices
are retained. They must resolve to one unblocked surface, and an interior's inside
anchor cannot override its shared virtual room. A compiler-generated fixture loads
in the native engine with both endpoint coordinates outside their linked polygons
while retaining the intended gate registrations. Recovery packet conversion
preserves the anchors. Transitions similarly support a local `waypointAnchor`, so
Lincoln's annex transition retains its reference point while linking the intended
landing. This does not certify full navigation connectivity across receiving areas.
Leicester's three drawbridges have shared multi-scene GLBs. Their initial/applied
views now share one gameplay definition per placement. Compilation unions local
state parts, deduplicates shared frames and supplies hidden frames for alternate
states when only the initial view was inserted. It preserves world transforms when
the group's pivot changes, including rotated and elevated placements, without
changing the saved editor scene. Conflicting shared frames and ambiguous separately
edited parts fail explicitly. All 59 non-lift doors, 16 shared interiors and five
door-linked patches now match Leicester's reference geometry and rules. Visual-state
and typed-mask export remain unfinished; these are geometry diagnostics.
Linked changing geometry can establish a door owner only when every obstacle has
one owner and all belong to the same asset. `doorStateOwnershipRecovery` records
this evidence for physical-grouping review; conflicting or missing geometry cannot
select an owner. Recovery also supports sight changes without navigation
changes when every referenced obstacle and door belongs to the same asset.
The 60 recovered initial/applied sight references match source coordinates at native float32 precision
and preserve their flags. All eight initial/alternate permission fields match the source for the 66
linked door references, and each binding retains its trigger direction.
Ordinary passages can connect to stair/lift surfaces in either direction without
becoming lift doors. This restores Lincoln's hall passages onto traversal surfaces.
All nine extracted map diagnostics compile and load; the native round-trip harness
applies/resets their 48 recovered transitions. It checks both
halves of door permissions as well as movement and sight state, including the
door-to-patch links for door-triggered transitions.
Door-linked patch coverage is complete in these diagnostics; visual effects,
navigation fidelity and publication remain unfinished.
Spatial ownership ties can be resolved by slicing solid geometry above the landing,
excluding supporting terrain and preserving disconnected concave pieces. This
restores 84 connection records without dropping previously recovered doors.
Across the nine diagnostics, all 397 non-lift doors now compile and match
source endpoints, click polygons, door types, active flags and initial/alternate
permissions. York's final four entrances belong to two shared interiors spanning
independent buildings. Authored passage sockets restore those rooms while keeping
each entrance with its own building. Inferred physical grouping remains marked for
review before publication; complete door coverage does not certify full gameplay parity.
`compare-door-geometry.ts SOURCE_JSON COMPILED_LEVEL_JSON` independently compares
non-lift door geometry/rules, shared-room membership and door-linked patch rules.
It accepts regenerated indices and equivalent polygon winding, but fails on missing
or extra records, regrouped rooms, changed permissions or mismatched trigger direction.
It does not certify receiving-area connectivity, lift behavior, sight changes or visuals.
Current compiled/source counts (no unexpected records in any map):

| Map | Non-lift doors | Shared rooms | Door-linked patches |
| --- | ---: | ---: | ---: |
| Croisement01 | 3/3 | 0/0 | 0/0 |
| Croisement02 | 1/1 | 1/1 | 0/0 |
| Croisement03 | 5/5 | 0/0 | 1/1 |
| Derby | 42/42 | 14/14 | 1/1 |
| Leicester | 59/59 | 16/16 | 5/5 |
| Lincoln | 59/59 | 19/19 | 9/9 |
| Nottingham | 100/100 | 45/45 | 7/7 |
| Sherwood | 5/5 | 5/5 | 0/0 |
| York | 123/123 | 74/74 | 5/5 |

York's counts use the current library building groupings. Explicit entrance
ownership distinguishes raised terrain from buildings, the bridge gatehouse from
its adjoining tower, and overlapping market-house projections. These declarations
recover ten entrances that were unresolved in the newly grouped scene. Its 72 jump
pairs and five door-linked patches still match, and native geometry construction
and transition apply/reset pass. Two additional shared-room declarations connect
the paired castle lodges through their curtain-wall passage and the market corner
shop with its adjoining gabled house. Recovery requires every source entrance to
have exactly one owner, validates pinned frames and connected sockets, and checks
that each socket touches its owner's wall geometry. These become local positions
and directions in the assets; source building and door indices remain offline.
Optional interior sockets retain a local 3D point and a facing direction. Opposing
sockets within the placement tolerance join rooms; unmatched sockets leave rooms
independent and ambiguous matches fail. Doorless connector assets participate in
joins but create no runtime room without an entrance. Compiler/export fixtures
and native engine tests cover joined and separated room registrations; rotated
and duplicated assemblies retain independent connections and door-transition links.
The recovery packet format preserves these definitions without global identifiers.
The original York arrangement passes native construction with all 123 doors,
74 rooms and 72 jump pairs. A one-pixel courtyard-wall move now compiles and loads
as a static diagnostic with 75 rooms: the two lodges become independent, keeping
their entrances. The wall-to-house jump becomes unavailable, leaving 71 jump pairs.
Unmatched jump sockets emit warnings; unused landing zones are omitted and remaining
zone references are rebuilt. Restoring the asset placement reconnects the jump.
Ambiguous matches and conflicting jump rules still fail. A ten-pixel move also
intersects a neighboring stairway and fails the existing traversal-connectivity
check; this verification does not establish arbitrary-placement or full visual parity.

The earlier twenty-one-transition recovery batch passed native initialization, apply and reset checks:
movement-state bits, obstacle-sector activation and sight flags change and restore.
Transition reference points may lie inside static blockers; they must still resolve
to a unique surface at the authored height. Doors resolve their optional receiving
anchors, or otherwise their endpoints, against unblocked surfaces. Jump landing
anchors likewise require an unblocked receiving position.
The ignored `recovered_asset_transitions_apply_and_reset_native_geometry` test uses
`ROBIN_ASSET_MAP_DIAGNOSTICS` to load the generated transition-bearing probes.
Diagnostic batches fail if any map fails or no maps are exported. Each run invalidates
the previous manifest and removes each map's stale output before attempting recovery;
native checks reject failed entries instead of silently skipping them. A successful
batch still proves only the explicitly checked static geometry and state behavior.
Croisement03's elevated navigation-only change has approximately 24.49 square
pixels inside its movement area but outside every receiving surface. The reviewed
transition-plane recipe described above now supplies its boundary height without
adding receiving coverage. Its later southwest assembly recovery brings that
map's movement-group recovery to nine of nine; mask and visual state remain incomplete.
`omittedMovementTransitions` makes missing transition definitions explicit
in the static diagnostic and prevents it from certifying full compilation.
Ground recovery uses fixed-point polygon operations and reports reconstruction
area differences; generated boundaries are normalized after integer rounding.
Solid/surface intersections use fixed-point clipping. Redundant straight-edge
vertices are removed before rounding to avoid artificial navigation seams.
Recovered clearances retain the free-space boundary and extend only around their
owner's bounds; they can subtract that owner's collision, never another asset's.

Non-rendering gameplay volumes can attach to an existing asset frame without a
mesh. One-time recovery uses explicit catalog ownership (or `--ownership`) and
restores six of York's nine inventoried records against the published scene,
including a missing jump landing surface. Staged canonical tower, golden timber
house and stone-shop assets resolve the remaining three records; all nine now
have asset owners. The stone shop uses an explicit split that preserves its
neighboring building parts in a separate asset with unchanged geometry.
The updated ownership catalog additionally requires complete green timber-house
and striped-awning building assets; staging both restores their two local volumes
without assigning either volume to a partial building. The combined York draft
now constructs 194 movement areas, 1,161 sight obstacles, 244 doors and 72 jump
pairs, whose geometry and traversal flags match the source pairs.
The compiler reads
only the resulting local volumes; source sector and material indices are rejected.
`stage-canonical-static-asset.ts` combines complete static assets only when their
parts exactly match an explicit catalog group. It checks unchanged world collision
positions and decoded model geometry, materials and texture bytes after writing
the merged model. Partial groups require `--split`, which partitions leaf parts
without changing their world transforms, collision coordinates or appearance.
Merged and split models normalize their hierarchy to one Z-up map wrapper and
one identity asset group; the part transforms retain the placed geometry.
The staged index includes each new descriptor and model hash, and other map scenes
remain available in the overlay. The five York canonical assets and the split
remainder pass real-browser loading, insertion, save/reopen and rendering checks.
Every part must be assigned exactly once. Edited placements and state/gameplay
definitions requiring migration are rejected. Its output is a separate library overlay and
pinned editor scene, not a publication or a runtime dependency on source levels.
Jump recovery now requires an owned receiving surface on each elevated side;
missing or ambiguous ownership remains an explicit gap rather than an invalid pair.
Cross-asset edges retain only their own local landing zone and a shared geometric
socket. Compilation rejects missing or ambiguous mates and conflicting long-jump
rules; no source pair index or fixed scene reference links the assets.
Terrain owns ground jumps only between recovered terrain landing regions and
across retained terrain exclusions; any transferred asset-owned exclusion in the
jump corridor prevents that assignment. Split-asset ownership uses each actual
part footprint, allowing an edge to span multiple planes of one asset while
rejecting gaps between them. York's staged stone-shop roof resolves the final two
pairs. Recovery coverage is not a connectivity parity proof.

The physical-volume audit also checks whether visual component bounds preserve
the shared sight footprint and ordered bottom/top planes. Twenty-six split
records across Lincoln, Nottingham, Sherwood and York differ in footprint or
height; these are not certified equivalent merely because every record has an
asset owner. The audit is `work/map-compile/sight-partition-audit.json`.

York record 650 is wholly owned by the dedicated west-market shared occlusion
asset. Its five visual component bounds add about 1,154 square game units and
change the height planes. A reviewed `physical_volume_sources` declaration now
restores one asset-local volume and disables collision from the component bounds.
Recovery checks source/model hashes and exclusive ownership of every physical
part; receiving geometry or material regions require separate authoring. The
compiler reads only the resulting local definition, which moves with the asset.
The visual model remains intact. Other split records span separate assets and
still require ownership and geometry work; they cannot use this whole-asset fix.

`work/map-compile/york-whole-volume-recovery` retains the twenty reviewed masks.
The baseline and translated drafts reproduce the shared volume's ordered points
and flags at engine precision and construct 192 movement areas, 1,197 sight
obstacles, 254 doors and 72 jump pairs in Rust. Duplicating the complete asset
retains the first volume and adds exactly one independently translated volume;
the resulting 1,198-obstacle draft also constructs in Rust. Unit tests cover
rotation and duplication through the recovery-to-compiler path.
This is scoped geometry validation;
808 York masks and other previously listed gaps remain pending. No asset or map
is certified or published by this recovery.

Nottingham's front-market record 12 now has an explicitly reviewed partition
recipe in `refinement/catalogs/nottingham-market-volume-partitions.json`.
`pipeline/src/author-volume-partitions.ts` verifies source, model and descriptor
hashes, assigns every owner once, and writes independent draft descriptors. It
preserves the outer contour and constant bottom/top heights while removing the
east-green stall's extra collision across a notch. The four original meshes and
all other physical parts remain unchanged. Sloped, receiving, material-linked,
mask-linked and changing volumes require separate authoring and are rejected.

The reopened overlay is `work/map-compile/nottingham-market-volume-stage-v3`;
its recovered definitions are in `nottingham-market-volume-recovery`. The four
compiled pieces differ from the reference footprint by 0.000056 square game
units under fixed-point clipping, compared with roughly 2,512 extra square units
before correction. All bottom heights are exactly zero and top heights match at
engine precision. The baseline constructs 114 areas, 671 sight obstacles, 172
doors and 38 jump pairs in Rust. Each stall also compiles and constructs when
independently moved one unit east, with the other three volumes unchanged.
Those movements separate navigation sockets and therefore change connection
counts; they are placement checks, not baseline connectivity parity claims.
Trial translations of 100 units east/south blocked nearby entrances and were
rejected without suppressing collision. These separated-volume drafts did not
establish sight-query equivalence across partition seams; see the assembly check
below. Visual parity and publication remain unverified. These drafts
retain sixteen reviewed static masks; 507 Nottingham masks remain pending in
this recovery. The diagnostic manifest is `nottingham-market-volume-native`.

Native ray checks confirmed that an artificial partition face blocks a ray whose
endpoints are both inside the shared volume, while the complete volume leaves it
clear. Asset parts now support directed, local `sight_join_edges`. Compilation
joins matching placed edges only for compatible flat, static volumes; unmatched
edges leave independent pieces. Ambiguous matches, overlapping pieces, holes,
different flags/heights, or receiving/material/mask/state links are rejected.
Movement geometry stays owned by each asset. Sight references on unrelated masks
and transitions are rebuilt after joining, and duplicated or moved assets match
only their current geometric neighbors. No map identifiers or source indices
participate in seam matching.

The Nottingham authoring recipe now emits these seams. The reopened
`nottingham-market-volume-seams-stage` overlay and its `-seams-recovery` definitions
compile the four touching stalls into one volume with the reference vertices at
engine precision. The baseline constructs 114 areas, 668 sight obstacles, 172
doors and 38 jump pairs. Four independently moved drafts also construct, leaving
669 or 670 sight volumes as their seams separate. The native
`recovered_sight_assembly_preserves_native_ray_queries` diagnostic compares 100,000
deterministic rays against the reference volume, including impact presence,
coordinates and ray parameter; all match exactly. Its input is
`nottingham-market-volume-seams-native/sight-query-case.json` under
`work/map-compile`. This verifies sampled queries against this assembled volume,
not full-scene impact ordering, all placements, visual behavior or map parity.

The southwest parapet's flat record 219 uses a second reviewed partition recipe,
`refinement/catalogs/nottingham-southwest-parapet-volume-partitions.json`. Its north
and south assets now meet at the true bend, retain the southern inner corner and
have exactly ground-level bottoms. Their former component bounds added about
9,470 square game units and raised the bottoms by 0.00035–0.00044 units. The new
assembled footprint has zero difference under the clipping audit, and its eight
vertices match the reference at engine precision. Another 100,000 native sight
and impact queries match exactly.

`stage-volume-partitions.ts` stages draft descriptors in a new library overlay,
preserving model/resource paths and unchanged files. It verifies draft hashes,
input model/descriptor pins and the asset index, rejects conflicting per-instance
collision overrides, updates scene/index hashes and reopens the compact scene.
Its tests verify unchanged source files and model bytes, reload fidelity, stale
pin rejection, override rejection and exclusive creation of the output directory.

The combined market/parapet overlay is `work/map-compile/nottingham-parapet-volume-stage`;
recovery and native diagnostics use the matching `-recovery` and `-native`
directories. The baseline constructs 114 areas, 667 sight obstacles, 172 doors
and 38 jump pairs. Moving both wall assets and the attached southwest stair one
unit west also constructs, with 171 doors and 37 jump pairs as external sockets
separate. After correcting comparison of equivalent receiving planes, moving only
the north wall east now reaches the separate stair's unsupported landing check;
moving it west also leaves that landing unsupported. Those failures remain
explicit. This does not certify arbitrary detached wall/stair placements,
receiving-surface parity, appearance or publication.

York records 213 and 876 now have reviewed partition recipes in
`york-arcade-volume-partitions.json` and
`york-precinct-parapet-volume-partitions.json`. Each follows the existing component
seam, projected onto the exact outer contour. Record 213 restores ground-level
bottoms beneath the arcade/gallery volume instead of the roughly 89.9-unit gap
in component bounds. Record 876 restores the bastion's inner arc instead of
filling it, and corrects its slightly negative bottoms and raised top. Both retain
independent assets and assemble only when their local seams coincide.

The combined overlay is `work/map-compile/york-flat-volume-stage`, with definitions
in `york-flat-volume-recovery` and diagnostics in `york-flat-volume-native`.
It also retains the earlier correction for record 650. The baseline constructs
192 movement areas, 1,195 sight obstacles, 254 doors and 72 jump pairs. Each of
the four assets also constructs after an independent one-unit eastward move,
with 1,196 sight obstacles. Moving the arcade house separates two external door
and jump connections; the other three tested placements retain baseline counts.
Each assembled volume reproduces its reference vertices at engine precision and
passes 100,000 exact native sight/impact comparisons. The twenty reviewed York
masks remain present; 808 masks and the other listed gaps remain pending.
These are draft geometry/query checks, not publication or complete map parity.

Some scene-pinned York assets are absent from the older palette index. Partition
staging now registers those verified descriptors in the new overlay's index;
duplicate entries or conflicts with existing scene pins still fail. Tests verify
that the source index remains unchanged. Five of the twenty-six audited split
records now have correction drafts; the other twenty-one remain uncorrected in
that audit, including receiving volumes and more complex ownership cases.

Lincoln records 99 and 110 now have reviewed recipes in
`lincoln-southeast-parapet-volume-partitions.json` and
`lincoln-south-parapet-volume-partitions.json`. The existing turret/curtain and
bastion/curtain seams coincide with boundary vertices, so each asset retains an
exact portion of the contour and explicit local joining edges. Record 99 now
runs from ground to 363.001 instead of component bounds at 350–380. Record 110
runs from ground to 340.001 instead of bottom 320 and mismatched tops 355/364.
Their assembled reference vertices match at engine precision, and each passes
100,000 exact native sight and impact comparisons.

The overlay is `work/map-compile/lincoln-parapet-volume-stage`; recovery uses the
matching `-recovery` directory and retains four reviewed masks, with 424 still
pending. The baseline constructs 113 areas, 567 sight obstacles, 89 doors and 10
jump pairs. All four assets independently moved one unit east now compile and
construct in Rust. The corner turret produces 569 sight obstacles; the other
three produce 568, with unchanged door/jump counts throughout. The complete
successful batch is `lincoln-parapet-volume-native`. The baseline-only query
inputs are also retained under `lincoln-parapet-volume-baseline-native`.
Visual parity and publication remain unresolved. Seven of the twenty-six audited
split records had correction drafts at this stage; the stacked-volume work below
brings that count to nine.

Receiving-material conflict checks compare the native binary32 height-plane
coefficients after winding correction, rather than requiring identical anchor
coordinates. Moving a flat surface can change its anchors without changing its
runtime plane. The compiler retains authored anchors and still rejects different
materials or coefficients, including signed-zero differences. Regression tests
cover these distinctions; 10,000 deterministic flat/sloped triangles matched Rust
initialization bit-for-bit using `editor_receiving_plane_coefficients_match_native_initialization`
and `ROBIN_PROJECTION_PLANE_CASES`. This fixes the false Lincoln conflicts without
claiming complete receiving coverage, traversal or map parity.

To reproduce the coefficient comparison, run
`node --test shared/src/native-projection-plane.test.ts` from `level-editor` with
`ROBIN_PROJECTION_PLANE_CASES` set to an absolute output JSON path. Then use the
same environment variable from the repository root with
`cargo test -j 1 -p robin_engine --lib editor_receiving_plane_coefficients_match_native_initialization -- --ignored --nocapture`.

Lincoln records 104 and 127 now have reviewed stacked partitions in
`lincoln-door-turret-east-volume-partitions.json` and
`lincoln-door-turret-west-volume-partitions.json`. Existing component ownership
places the curtain-wall fill below height 320 and the separate cone turret above
it. Each piece retains the complete ordered concave footprint, with wall collision
from ground to 320 and turret collision from 320 to 373.001. This restores missing
ground-level collision and removes the enlarged component footprints. Visual
meshes remain unchanged; their appearance is not certified by this correction.

Asset parts opt into whole horizontal joins through `sight_join_caps`. The compiler
assembles matching top/bottom faces only when footprints, native heights and flags
agree, preserving the outer contour and removing the internal face. Detached
pieces remain independent. Mixed edge/cap joins, ambiguous placements and linked
receiving/material/mask/state volumes remain rejected. Authoring requires complete
height coverage without gaps or overlap. Regression tests exercise rotated stacks,
copies, detached caps and invalid definitions.

The recipes apply sequentially to `lincoln-parapet-volume-stage`, first producing
`lincoln-door-turret-east-draft`/`-stage`, then `lincoln-door-turret-west-draft` and
the combined `lincoln-door-turret-volume-stage`. Recovery and native diagnostics
use the combined name with `-recovery` and `-native`. The baseline reconstructs
both volumes' ordered vertices and flags at engine precision, and each passes
100,000 exact native sight/impact ray comparisons. Native construction passes for
the baseline (113 areas, 565 sight obstacles, 89 doors, 10 jump pairs), the wall
moved one unit east (114/567/89/10), and the turret independently moved one unit
east (113/567/89/10). Four reviewed masks remain recovered, with 424 masks pending.
Nine of the twenty-six audited split records now have correction drafts; seventeen
remain. Publication, full state behavior, visual and actor traversal parity are
still incomplete.

Sherwood record 24 now has explicit physical ownership in
`sherwood-ladder-oak-physical-owner.json`. Its single four-point trunk volume was
previously expanded into 38 component obstacles: four tree components and 34
ladder-platform components. `author-owned-volume.ts` restores the complete ordered
trunk volume on the oak's trunk frame and marks the other 37 parts with
`collision: "none"`. Their meshes, editor bounds and visual provenance remain
intact. This is independent of the ladder platform's own traversal geometry and
volume 101. Source/model/descriptor hashes and an exhaustive component list guard
the one-time authoring step; linked or receiving volumes require separate migration.
Compiler validation rejects mask, state, material, receiver or movement links to
disabled component collision.

The input is `sherwood-canopy-post-stage`. Its stale palette descriptor hashes are
reconciled against the checked scene pins in an isolated `sherwood-owner-input-stage`
overlay; the source library remains unchanged. The corrected definitions and scene
are in `sherwood-ladder-oak-owner-draft` and `sherwood-ladder-oak-owner-stage`.
Matching `-recovery` and `-native` directories contain the recovered candidates and
native diagnostics. Baseline construction has 29 areas, 258 sight obstacles, 15
doors and one jump pair. Moving the oak 100 units east, the ladder independently
100 units east, or both together also constructs, with unchanged door/jump counts.
The trunk's ordered vertices and flags match at engine precision; 100,000 native
sight/impact comparisons match exactly. The recovery still has 166 pending masks,
one shadow region and five sound sources. No Sherwood masks are certified here.
Ten of the original twenty-six split records now have correction drafts; sixteen
remain, alongside publication, visuals, states and traversal verification.

Sherwood record 102 now has a reviewed physical owner in
`sherwood-central-treehouse-physical-owner.json`. Its six-point concave hut body,
including the doorway notch, spans heights 284.001–351.001. The complete volume
belongs to the treehouse's first wall-plank frame. Ninety-three other wall planks
and twenty-two platform rails, posts, rungs and braces retain their visuals with
collision disabled. The separate platform retains its walking surfaces and
traversal volumes 97/98; the treehouse retains its interior entrance.

Apply `author-owned-volume.ts` to `sherwood-ladder-oak-owner-stage` using this recipe,
then stage into `sherwood-central-treehouse-owner-stage`. The corresponding
`-draft`, `-recovery` and `-native` directories contain authoring output, recovered
candidates and native checks. Baseline construction has 29 areas, 143 sight
obstacles, 15 doors and one jump pair. Moving the treehouse one unit east, the
platform independently one unit east, or both together 100 units east preserves
those counts and constructs successfully. All 48 directed lift callbacks across
the four cases preserve sectors/layers. Record 102 also passes 100,000 exact native
sight/impact comparisons. Recovery still has 166 missing masks, one shadow region
and five sound sources.

All 127 source Sherwood obstacle records now have exact ordered vertex/flag matches
at binary32 precision in the baseline. Sixteen additional records are non-solid,
non-opaque receiving surfaces. This does **not** prove full-scene collision parity:
`recovered_scene_preserves_native_sight_and_impact_queries`, using the baseline
`sight-scene.json` through `ROBIN_SIGHT_SCENE_CASE`, finds zero sight differences but
1,298 impact-position differences across 200,000 deterministic solid/opaque
queries. The compiler changes the order of 126 of the 127 matched records; native
impact grouping depends on candidate order. A diagnostic-only control,
`sight-scene-ordered-control.json`, changes only the compiled obstacle order to
match the source order and produces zero differences across the same 200,000
queries. This confirms the ordering cause; it is not a compiler fix or an allowed
source-dependent export path. The diagnostic deliberately remains
failing for this export. It compares full obstacle lists without a fast-find grid,
and does not certify receiving layers, mouse selection, materials or actor routing.
Eleven of the twenty-six audited split records have correction drafts; fifteen
remain. The ordering correction below addresses the demonstrated impact gap.

`AssetGameplay.sightOrder` now maps local part/volume IDs to explicit query
precedence. Offline recovery writes this metadata into each asset candidate;
compilation reads only these asset definitions. It orders the final physical
volumes after assembly and rebuilds every mask and initial/applied sight-state
reference. Joined pieces must agree on explicit precedence. Equal values retain
placement order, while unranked authored volumes retain their relative order
after ranked volumes. Movement and duplication carry the metadata with the asset.

The corrected Sherwood candidates are in `sherwood-ordered-owner-recovery`, with
compiled diagnostics in `sherwood-ordered-owner-native`. The actual compiler output
now passes the same 200,000 whole-scene solid/opaque queries with zero sight or
impact differences, without diagnostic reordering. Baseline and all three moved
treehouse/platform cases construct 29 areas, 143 sight obstacles, 15 doors and
one jump pair in Rust. Tests also cover remapping masks/state links, stable ties,
invalid asset references and conflicting assembly precedence. This closes the
observed full-list impact-order gap; fast-find-grid candidate behavior, material
ties, receiving geometry, visuals and full actor traversal still need verification.

The metadata migration also validates and compiles for all nine maps with source
level data. `work/map-compile/query-order-recovery/validation.json` records their
input libraries and zero invalid candidates. Local priority counts are
Croisement01 85, Croisement02 150, Croisement03 106, Derby 271, Leicester 392,
Lincoln 474, Nottingham 565, Sherwood 127 and York 993. These count independently
authored physical pieces before assembly. Compiled static drafts are in
`work/map-compile/query-order-native`; Wychford remains outside this source-backed
verification. All nine drafts pass native geometry construction with their rebuilt
mask and state references. This migration does not certify the other maps' remaining geometry,
state, mask or full-scene query differences.

Sherwood's scene-query diagnostic now also constructs native fast-find grids.
`sherwood-ordered-owner-native/sight-grid-scene.json` supplies 30×17 map cells
(1920×1088 game units), plus the source and compiled conventional layer counts.
Each obstacle is registered with its ground bounds and optional receiving layer;
the native grid adds its normal padded rows and special layers. Across 200,000
solid/opaque segment queries, 164,460 produce nonempty candidate lists. Ordered
candidate lists and resulting impact positions match exactly. The same run also retains the full-list sight/impact
comparison. This verifies obstacle indexing and candidate order for the sampled
Sherwood rays, not navigation graph connectivity, mask queries, mouse selection,
material ties, receiving-height queries, world-boundary exits or complete fast-find-grid parity.

Sherwood's reviewed `refinement/catalogs/sherwood-projections.json` now restores
14 physical receiving-volume links. The offline recovery validates unique ownership,
ordered binary32 geometry, flags and model/source pins before replacing generated
receivers with asset-local volume references. Traversal receivers 97, 98 and 101 were
already linked. Ground bluff 111 remains excluded because its surface spans two
compiled receiving areas. The compiler still consumes only the scene and assets.
The candidates in `work/map-compile/sherwood-physical-receiver-recovery` compile to
129 sight volumes instead of 143. Baseline, treehouse-only move, platform-only move
and combined move all construct natively with 29 areas, 15 doors and one jump pair.

The native projection diagnostic now accepts a source geometry fixture through
`before_proto`, supports different compiled layer numbers, and reports differences
per sector pair with the selected obstacle indices. Comparing the 14 restored
receivers in `work/map-compile/sherwood-physical-receiver-native` samples 701,438
integer/half-pixel positions: seven cases match exactly; the other seven have
19,664 coverage differences and 27 height differences (maximum 1.9894714), with
zero material differences. That batch fails the parity assertion.
All differences belong to source receiving area 31, which that batch
splits across bridge/platform areas. Samples cover each receiver's bounding rectangle,
so differences can include neighboring receivers in the shared source area; these
counts are not a count of missing physical polygons. The recovered boundaries have
gaps and height offsets at bridge landings, preventing exact 3D edge joins without
further navigation authoring. This is partial receiving recovery, not full map parity
or publication of the asset definitions.

The ignored native test accepts optional `grid_size`, `source_layers` and
`compiled_layers` alongside its `source`/`compiled` obstacle arrays through
`ROBIN_SIGHT_SCENE_CASE`. Grid sizes are in 64-unit cells, not pixels. It also
requires nonempty candidate lists to ensure the grid path is exercised.

The subsequent `sherwood-navigation-joins.json` recipe restores the complete
treehouse movement contour as seven asset-local planar pieces, retaining all three
holes and the independent receiving footprints. Six explicit bridge/platform seams
join those pieces. An eighth existing surface shares its owner's local region.
Ordinary sockets still require coincident 3D edges by default; an asset may explicitly
allow a maximum height step via `navigationJoinHeightTolerance`. Both sides must
permit the step and their projected endpoints must coincide. Same-side overlaps,
ambiguous matches and same-owner joins remain errors. These Sherwood seams allow
four game units (largest authored endpoint step 3.6302); moving a socket away
detaches its navigation region. Receiving heights and geometry are not flattened.

Reviewed recovery can now replace a surface's movement contour while retaining its
height plane. It generates owned collision clearances from the restored contours
before other geometry recovery; the compiler needs no source map. The authored
piece union exactly reproduces the source movement polygon and its holes before
compilation. In `work/map-compile/sherwood-navigation-native`, all 701,438 native
receiving samples now match exactly: zero coverage, height or material differences.
Baseline and treehouse-only movement construct 23 areas; central-platform-only and
combined movement construct 24. All four have 129 sight volumes, 15 doors and one
jump pair. The first compiled connected movement contour had 36.4714285714
square game units of symmetric difference in seven small boundary regions.

Clearance precision recovery now removes that discrepancy. Offline recovery retains
fractional clearance intersections, and the compiler clips those intermediate cutouts
before rounding the final movement boundaries. Straight-edge cleanup of generated
boolean output tolerates two fixed-point clipping units, preventing numerical noise
from turning a redundant intersection into a whole-pixel kink. Authored surface
validation remains strict. The platform-92 recipe also removes a rounded intersection
that extended its redundant movement surface into a source movement hole.
The rebuilt `sherwood-navigation-native` baseline has zero polygon symmetric
difference for this connected region: the same 66 outer vertices and all three holes.
The 701,438 sampled native receiving queries still match exactly. Ground bluff 111,
other navigation areas, full actor traversal, visuals and publication remain open;
this does not certify full map parity.

The precision change was also applied through fresh recovery of all nine maps in
`work/map-compile/clearance-precision-recovery`: every asset candidate validates,
and all nine static exports in `clearance-precision-native` construct in Rust.
The resulting area counts are 46/32/30 for the crossings, 60 Derby, 82 Leicester,
112 Lincoln, 114 Nottingham, 29 Sherwood and 198 York. This broad batch uses the
existing query-order authoring configuration, without the separate Sherwood physical
receiver/navigation recipes. Counts changed on several maps, including three extra
York receiving records; those topology changes still require source comparisons.
Native construction is a regression check, not a full parity certificate. The
dedicated Sherwood navigation batch additionally passes all 48 directed lift callbacks.

Ground decomposition now retains the fixed-point clipping grid instead of snapping
terrain and asset-owned blockers separately. Recovered ground definitions explicitly
set `preserveMovementPrecision`; the compiler combines their fractional boundaries
before snapping the final movement regions. Existing asset definitions keep their
previous rounding behavior unless they opt in. Near-collinear clipping noise is
removed before storing ground rings, without rounding their remaining coordinates.
Sherwood's offline ground decomposition error falls from 479.3164548 to about
0.00004992 square game units. The dedicated `sherwood-ground-precision-native`
baseline reduces separate flat-ground areas from eleven to four and reduces their
polygon difference against the source ground minus its raised footprint from
442.8548631 to 78.9977270 square game units. Combining the compiled flat ground and
the bluff's two sloping pieces still leaves 73.6571309 square units of difference
against the complete source ground area and four disconnected components. This does
not yet connect the river bluff or permit its physical receiver link. The treehouse
region remains geometrically exact and all 701,438 native receiving samples still
match. These are unpublished recovery candidates, not a full-map parity result.

Fresh all-map recovery in `work/map-compile/ground-precision-recovery` reduces the
ground-decomposition difference below 0.0006 square units on eight maps. Croisement01
improves from 1023.3774 to 66.4752 square units but retains a larger discrepancy.
All candidates validate and all nine static descriptors in `ground-precision-native`
construct in Rust: 40/20/19 areas for the crossings, 60 Derby, 68 Leicester,
108 Lincoln, 103 Nottingham, 22 Sherwood and 190 York. These counts use the broad
authoring configuration without the dedicated Sherwood receiver/navigation recipes.
The dedicated Sherwood variants construct 16 areas at baseline/treehouse-only move
and 17 for platform-only/combined moves; all four retain 129 sight obstacles,
15 doors, one jump pair and passing directed lift callbacks. Recovery error and
successful construction do not establish final navigation or full gameplay parity.

Croisement01's remaining 66.4752-square-unit ground-recovery discrepancy came from
reconstructing exclusions as the complement of already clipped free space. A second
boolean operation erased a narrow corridor between overlapping exclusions. Recovery
now clips the authored exclusion contours directly to the ground boundary before
transferring asset-owned cutouts. Its decomposition difference is now 0.0001773
square units. A reduced regression with three overlapping triangular exclusions
checks this case; the previous complement reconstruction lost about 23.15 square
units in that fixture. The fix changes offline asset authoring, not the compiler's
source-data isolation or the separation between maps and missions.

The fresh `work/map-compile/ground-exclusions-recovery` batch has valid asset
candidates and ground-decomposition differences below 0.0006 square units for all
nine maps. All nine static descriptors in `ground-exclusions-native` compile and
construct in Rust. Croisement01 now constructs 41 areas instead of 40; the other
eight area counts are unchanged from the ground-precision batch. This verifies
offline decomposition and native loading, not complete final navigation parity or
publication of the recovered definitions.

### Preserved ground-boundary recovery draft

The opt-in `--preserve-ground-boundaries` authoring path retains one outer
movement envelope per source area. It transfers only authored obstacle coverage
to placed assets, including portions crossing the envelope, and stores remaining
exclusions in the terrain asset. Export still reads only asset metadata.
Compound exclusions are partitioned with a symmetric-difference check. Recovery
can split nearly touching fractional holes before assembly; final obstacle
partitioning does not add fractional movement vertices. A regression covers a
hole that triangulation previously filled silently.

The dedicated `sherwood-boundary-native` draft loads in Rust at the baseline and
three independent treehouse/platform placements. Its baseline outer ground
contour matches all 115 authored vertices exactly. All 701,438 receiving queries
still match coverage, height and material. This draft is **not an improvement in
overall ground coverage yet**: flat-ground symmetric difference is about 204.26
square map units versus about 79.00 in the preceding precision draft, after
accounting for the separately recovered raised bluff. Merging overlapping
exclusions before integer rounding remains unresolved. This mode stays opt-in;
the recovered definitions are not published or parity-certified.

The `ground-boundary-recovery` batch produces valid asset definitions for all
nine maps. Its `ground-boundary-native` export batch remains incomplete: Lincoln
and Nottingham fail the obstacle-partition coverage guard (about 25,600 and 357
square units respectively), and York reports a disconnected garden-wall lift
assembly. The three crossing maps, Derby, Leicester and Sherwood compile.
These failures are retained in the full diagnostic manifest; they are not
converted into successful empty geometry or omitted from the batch result.
The six successful exports also construct in Rust. The separately labelled
`ground-boundary-successful-native` subset records that loader check without
marking the nine-map batch complete. A source-contour diagnostic produces about
100.92 square units of error from union-and-rounding alone, before asset ownership
splits or physical collision cuts; obstacle-intersection preservation therefore
needs its own treatment in addition to the outer-boundary work.

The Lincoln and Nottingham partition failures above were subsequently traced to
false differences from fixed-point XOR on coincident triangle edges around
narrow holes. Partition verification now unions the triangles and compares
coverage with the floating-point polygon operation, retaining the real
nearly-touching-hole failure regression. The reduced integer regression has zero
symmetric difference and introduces no fractional vertices. Both fresh exports
in `ground-boundary-partition-native` construct in Rust: Lincoln has 82 areas,
565 sight obstacles, 89 doors and 10 jump pairs; Nottingham has 102 areas,
667 sight obstacles, 172 doors and 38 jump pairs. This resolves those two export
failures, without certifying their map fidelity. York's lift failure and the
overlapping-obstacle rounding differences remain unresolved.

### Explicit empty movement ownership

Boundary recovery now retains an explicit empty `movementBlockers` list when
an asset footprint overlaps the ground envelope but owns no authored exclusion.
Omitting that field enables model-derived movement collision, which had clipped
York's garden-wall stair into disconnected pieces. Empty authored ownership
keeps sight geometry intact and prevents that unintended fallback.

The fresh `york-boundary-empty-native` draft compiles and constructs 178 areas,
1,198 sight obstacles, 254 doors and 72 jump pairs in Rust. All 170 directed lift
passage callback checks pass; approach routing and animation remain separate
checks. The dedicated `sherwood-boundary-empty-native` baseline and three moved
treehouse/platform cases compile as well. Its outer ground contour remains exact;
flat-ground difference is about 185.70 square units, down from 204.26 but still
above the earlier precision draft. Boundary recovery remains opt-in and the
definitions remain unpublished.

The regenerated `ground-boundary-empty-recovery` batch has valid candidates for
all nine maps. All nine `ground-boundary-empty-native` descriptors now compile
and construct in Rust, including York. The complete static-loading manifest
therefore supersedes the earlier three export failures; it does not certify
navigation fidelity, missing visual/state coverage, or publication.

### Independent exclusion contours

Preserved movement boundaries now accept `holeContours` alongside their holes;
explicit asset movement blockers can carry matching `movementContour` labels.
Labels identify fragments that should be unioned after placement and before
integer rounding. Different labels retain separate obstacle contours, preserving
their implicit fractional intersections. Labels are shared authoring metadata,
not runtime sector indices; recovery namespaces them by the terrain asset.
Unlabelled exclusions retain their existing union behavior. Moving a fragment
uses its transformed local geometry and does not restore its earlier placement.

The compiler-generated native overlap fixture verifies both a reachable strip
and an obstruction that disappear when overlapping contours are merged and
rounded together. Recovery retains each exclusion's separate ownership cuts in
the opt-in boundary mode. In the dedicated `sherwood-contour-native` draft,
flat-ground difference initially decreased from about 185.70 to 115.42 square
units. Fragment assembly now matches nearby endpoints within clipping-grid noise
before unioning same-labelled pieces, and removes microscopic backtracking spikes
before integer rounding. This removes the remaining 14.5 square units of assembly
kinks: comparison against independently rounded complete contours now has zero
difference. The roughly 100.92 square units from the separately represented sloped
bluff remain. Boundary recovery cannot yet replace the default or claim full-map
fidelity. Publication remains unfinished.

All nine fresh `ground-contours-recovery` candidate sets validate, and all nine
`ground-contours-native` descriptors compile and construct in Rust. Their area,
sight, door and jump counts match the preceding empty-ownership batch. The
dedicated Sherwood treehouse/platform placement cases also compile. These checks
establish export/loading and the targeted overlap behavior, not full-map parity.

Regression tests also retain small holes in state-dependent movement blockers
and material receivers. Two simplification callbacks previously received the
ring's array index as a distance tolerance, unintentionally erasing those holes;
they now use the strict default tolerance. Assembly tests cover reordered input,
rotation, separate labels and detached pieces, without modifying source geometry.
The nine-map compilation and native construction checks pass again after these
fixes, as do all four dedicated Sherwood placement cases.
The dedicated baseline also matches all 701,438 sampled Rust receiving queries
across the fourteen reviewed physical receivers: no height, material or coverage
differences. This comparison does not include the unresolved bluff receiver.

### Receivers independent of movement boundaries

Asset gameplay now supports `projectionReceivers`: each binding names a local
physical part/volume and a local 3D navigation anchor. The anchor selects one
unblocked ordinary navigation area after placement. Its elevation belongs to
that area's walking plane, independently of the receiver's physical top plane.
The binding generates no walking polygon or terrain cutout. Movement collision
remains separately controlled by the asset's movement definitions.

The compiler-generated `asset-anchored-receiver` fixture loads in Rust with one
uninterrupted ground area, while the physical slope supplies elevation through
native receiving queries. Editor tests move, rotate and duplicate the receiver
without changing ground navigation, and reject dangling or conflicting links.
Offline authoring packets preserve these bindings as independent asset metadata.

The offline `--ground-receivers` recipe now migrates the Sherwood bluff into this
representation. It verifies source/model pins, unique physical ownership, ordered
binary32 geometry and flags, and a static unblocked ground anchor before writing
asset-local metadata. Recovery retains the full ground movement area instead of
subtracting the receiver footprint or generating replacement walking surfaces.

In `sherwood-anchored-ground-native`, the outer ground contour matches all 115
vertices and an independent polygon comparison reports zero walkable-area
difference. This resolves the previous approximately 100.92-square-unit error.
All 1,023,981 sampled Rust receiving queries across fifteen physical receivers,
including the bluff, match height, material and coverage. The baseline constructs
11 areas, 127 sight obstacles, 15 doors and one jump pair. The four treehouse
placement cases and a 50-unit bluff translation compile successfully.
All five descriptors construct in Rust, and all 60 directed lift passage
callbacks retain their expected sector and layer. These callbacks do not test
actor approach routing or traversal animations.

This is targeted ground/receiving evidence, not full-map certification. The draft
still has 166 pending masks, one light region and five sound sources, and full
actor traversal, related receiving anchors and publication remain unfinished.

### Ground receiver recovery across maps

Pinned recipes now cover 33 uniquely owned physical receivers: Croisement03 (3),
Leicester (5), Lincoln (19), Sherwood (1) and York (5). Nottingham's two candidate
receivers have no unblocked integer anchor inside their footprints and remain
unmigrated. Export resolves feature anchors at a bound receiver's elevation while
retaining the same navigation sector; a regression checks a sloped passage and
rejects an elevated anchor outside the receiver footprint.

The nine-map `ground-receivers-native` batch compiles and constructs in Rust.
Leicester's five migrated receivers match all 6,180,807 sampled native height,
material and coverage queries. The comparison harness now creates empty tenant
records only for actual buildings, excluding standalone door groups. These are
test-harness mission records, not content added to exported maps.

Restoring receiver footprints also requires restoring ground collision clearance
coverage. Without that, nearby physical parts introduce blocked ground despite
the shared navigation binding. After this correction, the
`ground-receivers-clearance-native` comparison reports:

| Map | Matching static ground areas | Remaining walkable-area difference |
| --- | --- | --- |
| Croisement03 | 1/1 | 0 |
| Derby | 4/4 | 0 |
| Leicester | 10/10 | 0 |
| Lincoln | 8/8 | 0 |
| Nottingham | 9/9 | 0 |
| Sherwood | 1/1 | 0 |
| York | 7/7 | 0 |

This compares polygon coverage, allowing removal of collinear vertices. It covers
all 40 static ground areas exactly; Croisement01/02 have no areas meeting this
static-ground filter. Stateful navigation and other layers require separate
verification. Only Lincoln and York have been regenerated with the clearance
correction in this batch; the other maps retain their earlier receiver drafts.
Map publication, stateful navigation and full gameplay parity remain unfinished.

The remaining York slivers came from rounding generated collision contacts lying
outside a ground envelope. Preserved-boundary compilation now tests overlap
before rounding, removing clipping-grid noise from fractional contacts with the
same tolerance used by generated motion cleanup. Complete integer contours keep
their implicit fractional intersections. Regression tests distinguish outside
contacts, one-grid-unit noise and genuine inward overlap. The resulting nine-map
batch constructs in Rust and passes all 368 directed lift passage callbacks;
this checks sector/layer changes, not actor approach routing or animations.

The earlier `ground-receivers-native` Lincoln draft also completed all 19,052,393
receiving queries with zero height, material or coverage differences. That query
result belongs to the saved draft before the clearance/contact fixes; current
ground coverage is verified separately by the comparison above.
On the updated clearance/contact batch, York's five migrated receivers pass
2,358,053 queries, Croisement03's three pass 54,507, and Sherwood's bluff passes
322,543, all with zero height, material or coverage differences.

### State-dependent movement contours

Preserved movement areas now retain complete initial/applied obstacle contours,
including parts crossing the outer boundary or permanent exclusions. They still
require genuine overlap with walkable coverage before allocating a state pair.
Other receiving-plane partitions retain their existing clipping behavior.
The compiler-generated `asset-preserved-state-boundary` fixture verifies a narrow
fractional route in Rust through the initial, applied and reset states.

This removes three measured contour discrepancies: Croisement02 patch 6's
applied blocker (about 0.494 square units), Croisement03 patch 0's initial blocker
(about 9.489), and Croisement03 patch 5's applied blocker (about 0.102).
The first independent state-coverage comparison found matching initial and
applied blocker coverage for 12 of 21 recovered movement-transition records on
matching source-area envelopes. The shared-receiver recovery below raises that
to 19 of 21; two still need comparison across separately compiled area partitions.
Six additional source
movement transitions remain unrecovered: five span assets, and one lacks an
explicit owner. These are not covered by the 40-area static-ground result.

All nine current draft exports apply and reset their 50 compiled transitions in
the Rust diagnostic, including door-only transitions. That verifies exported
bindings and runtime state changes; it does not prove coverage of missing source
transitions or equivalence of the remaining movement geometry.

### Shared receivers on stateful ground

Ground receiver authoring now accepts a persistent movement area with changing
obstacles, provided the selected anchor lies outside every initial and applied
obstacle. State geometry beneath these physical receivers is recovered on the
shared ground navigation plane; receiving height and material remain attached to
the physical volume. No source lookup is added to compilation.

The pinned Croisement01 and Croisement02 ground-receiver catalogs recover 15 and
8 receivers respectively. Croisement02 receiver 136 uses a nearby asset-local
navigation anchor outside its footprint, verified free in every movement state.
Recovered drafts compile from scenes and asset candidates alone.
All 13 recovered movement transitions across these two drafts now have identical
initial and applied blocked coverage on matching movement envelopes, removing
all four previously measured state differences and three envelope mismatches.
The Rust diagnostic applies and resets all 13 successfully.
Both drafts also construct native geometry. The initial permanent-coverage check
found about 31.028 square units of difference in Croisement02; the precise
ownership extraction below removes that difference.

This evidence is limited to recovered transitions. Each crossing map still has
one transition spanning assets that is not recovered. At that stage, Croisement03
and Nottingham each retained one unmatched recovered movement envelope and six
source transitions were missing; the current combined counts are at the top of
this document. The new catalogs and candidates do not
constitute published asset definitions or full map certification.

### Precise ownership extraction for shared contour edges

The one-time recovery command supports `--precise-ground-ownership` alongside
`--preserve-ground-boundaries`. It preserves floating intersections while
splitting contours among assets, postponing grid rounding until compilation.
Repeated extraction-grid operations previously separated coincident ownership
edges and produced whole-pixel kinks when the fragments were reassembled.
The recovery report records whether this mode was used; exported assets require
no special runtime mode and no source lookup.

Both crossing drafts use this option. Their matching ground envelopes now have
zero permanent-coverage difference, while all 13 recovered movement transitions
retain exact initial/applied blocked coverage. Native receiving scans pass
5,806,347 queries for Croisement01 and 3,759,568 for Croisement02 with zero height,
material or coverage differences. The latter includes the previously missing
receiver 136; before binding it, the scan found 540 coverage and three height
differences around that receiver.

This extraction mode is opt-in, not a globally certified replacement for the
fixed-grid authoring path. Trials on other maps exposed near-coincident polygon
failures and a Sherwood contour discrepancy; those maps retain their existing
recovery mode. The default compiler algorithm is unchanged. Neither these scans
nor the permanent-ground comparisons cover unrecovered transitions,
visual states, all traversal behavior, or asset publication.
Re-running the default recovery path for all nine source-backed maps produces
valid candidates and native geometry; its 40 previously verified static ground
areas remain geometrically exact.

### Combined Croisement03 state and ground recovery

The newer state-assembly scene must be retained when recovering ground metadata.
The older `projection-material-library` scene lacks the terrace navigation asset
and southwest state assembly, so reusing it loses two already recovered movement
groups. The current combined recovery uses the staged scene documented above,
the committed `croisement03-navigation-ownership.json` catalog, and separate
ground/elevated receiver catalogs. `croisement03-elevated-projections.json`
contains the eleven elevated entries; the three ground receivers must not also
be assigned replacement walking surfaces by the older fourteen-entry recipe.

From `level-editor`, recover the combined candidate with:

```sh
node pipeline/src/recover-asset-gameplay.ts \
  --library work/map-compile/croisement03-state-assembly-stage \
  --map work/map-compile/croisement03-state-assembly-stage/scenes/croisement03.rhlos-map.json \
  --source library/game-data/Data/Levels/Croisement03.rhp.json \
  --out work/map-compile/croisement03-combined-recovery \
  --ownership refinement/catalogs/croisement03-navigation-ownership.json \
  --transition-planes refinement/catalogs/croisement03-transition-planes.json \
  --projection-definitions refinement/catalogs/croisement03-elevated-projections.json \
  --ground-receivers refinement/catalogs/croisement03-ground-receivers.json \
  --mask-definitions refinement/catalogs/croisement03-masks.json \
  --navigation-definitions refinement/catalogs/croisement03-navigation-joins.json \
  --preserve-ground-boundaries --require-movement-coverage
```

`--require-movement-coverage` checks that every source movement group produced
an asset definition before creating output. The older scene fails this check
with two missing transitions. The report records whether this gate was requested;
passing it does not certify masks, visuals or complete transition behavior.

The combined baseline and an independent one-pixel assembly move both construct
in Rust and apply/reset all nine transitions. The moved assembly preserves its
four ordered physical shapes and flags and moves its waypoint. Seven ground
transition records have exact initial/applied coverage on matching source
envelopes; the two elevated transitions still require comparison across their
separate compiled areas. The static ground envelope remains exact. The candidate
still has 130 pending masks, six pending sound sources and unverified visual and
actor-traversal behavior, and remains unpublished.
The combined fourteen-receiver scan runs 792,816 native queries and finds 32,104
coverage differences around terrace receivers 52–54, which share one source
movement area but remain separate compiled areas. No height or material differences
occur where both sides return coverage. Eleven receiver cases (including all
three ground receivers) have zero differences; the terrace needs explicit
navigation assembly before its receiving behavior can be certified.

### Croisement03 terrace navigation assembly

`croisement03-navigation-joins.json` partitions the complete terrace movement
boundary between the terrace and its two access slopes. The placed polygons
reconstruct that boundary exactly. Asset-local edge sockets join only when their
projected endpoints coincide; the reviewed endpoint height steps stay below two
units at the west seam and four at the east seam. Receiving volumes and their
height/material definitions remain independent of the walking partition.

Transition fragments can now use `movementContour` labels to rejoin before final
integer rounding. Recovery supplies these labels and retains fractional movement
coordinates. Labels remain scoped to the placed transition and initial/applied
state, and distinct contours retain independent intersections. This removes the
8.5-square-unit terrace blocker discrepancy caused by separately rounded pieces.

The draft under `work/map-compile/croisement03-terrace-native` has matching initial
and applied coverage for eight of its nine transitions, including the terrace,
and retains exact static ground coverage. The fourteen-receiver native scan now
passes all 792,816 queries with zero coverage, height or material differences,
removing the 32,104 differences reported above. All nine transitions apply/reset
in the baseline, southwest-assembly move, and independent 20-unit eastward moves
of either access slope. Each detached slope keeps its physical receiver and
becomes a separate navigation area. Actor approach/traversal, the other elevated
transition's area coverage, visual states and publication remain unverified.

### Complete Croisement03 movement boundaries

Reviewed navigation recovery now also accepts a single physical owner with an
explicit planar boundary and no join sockets. This covers walking regions that
extend outside their receiving footprint without inventing additional receiving
geometry or a second owner. Model/source pins and local-plane validation remain
required, and a single entry without a boundary or with a join socket fails.

The navigation catalog restores the western platform's complete boundary and
the seven other single-receiver elevated boundaries. All eleven source movement
areas now have exactly one matching compiled envelope and zero difference in
permanent walkable coverage; the compiled baseline has no extra movement areas.
All nine transitions match both initial and applied obstacle coverage on those
envelopes. The fourteen-receiver scan still passes 792,816 native queries with
zero differences after the boundary restoration.

The baseline and the three moved/detached diagnostic scenes construct in Rust
and apply/reset all nine transitions. These checks establish geometry and state
binding fidelity for this draft, not complete gameplay parity. Actor routing and
traversal, masks, visual states, environmental sounds, and publishing the combined
asset definitions remain outstanding.

### Nottingham northern facade state assembly

`nottingham-state-assembly.json` assigns the four facade parts controlled by one
movement change to a single movable asset. Static splitting retains their complete
mesh subtrees, exact small transforms and component provenance, while recalculating
partition bounds. Declared owners cannot overlap and component references cannot
cross partitions. The remaining house structures retain their door ownership and
static mask support through `nottingham-state-ownership.json` and
`nottingham-state-masks.json`.

The staged scene is `work/map-compile/nottingham-state-assembly-stage`; recovery
uses those two catalogs, `nottingham-ground-receivers.json`, preserved ground
boundaries and the complete movement-coverage gate. The resulting candidate is
`work/map-compile/nottingham-state-assembly-recovery`. Compilation reads only
that scene and the recovered asset definitions.

The baseline and a one-pixel assembly move construct in Rust with 101 movement
areas, 666 sight obstacles, 172 doors and 38 jump pairs. Both apply/reset all nine
compiled transitions (two movement changes and seven door-only changes). The four
changing physical shapes retain their ordered vertices and flags, and their
waypoint follows the assembly. Source movement change 10 has exactly matching
initial and applied blocked coverage. Both movement changes are recovered, but
change 9 still needs navigation-boundary assembly before an envelope comparison
can pass.

The sloped receiver 168 stays attached to the shared ground navigation region.
Its Rust comparison passes 67,521 queries with zero height, material or coverage
differences. All twenty compiled mask records are unchanged from the preceding
draft. This does not recover the facade change's visual masks 378–384: 507 masks,
23 shadow regions and 18 sound sources remain pending in this candidate. Actor
traversal, visual-state fidelity and asset publication also remain unverified.

### Nottingham courtyard receiving assembly (movement fidelity pending)

`nottingham-state-navigation.json` partitions the courtyard and raised entry
walkway among their five physical surfaces. The four stair surfaces belong to
one asset and share a local region; only the three courtyard/stair boundary
edges need cross-asset sockets. The eastern seam allows an endpoint height step
below six units, and the other two allow less than one. These tolerances affect
socket matching only; the physical receiving planes retain their original values.
`nottingham-state-projections.json` binds all five physical volumes directly to
their surfaces, replacing synthesized receiving geometry.

The combined draft adds both catalogs to the northern facade recovery above.
Artifacts are under `work/map-compile/nottingham-state-navigation-recovery` and
`work/map-compile/nottingham-state-navigation-native`. Its six-receiver Rust scan
(the courtyard five plus ground receiver 168) passes 2,170,166 queries with zero
height, material or coverage differences. All twenty compiled masks remain
unchanged.

This is not movement parity. The courtyard's free coverage differs by about
7.921 square units initially and 17.748 after its change. The source keeps a
permanent obstacle crossing the outer boundary as a separate contour; this draft
clips it into the outer boundary and rounds intersections. Small extra corners
also appear along the raised walkway. Exact separate-contour handling across
joined navigation pieces is still needed. The overall verified transition count
therefore remains unchanged.

Recovery now rejects a join height tolerance without sockets before producing
asset packets, rather than emitting a packet that later fails gameplay validation.

### Preserved contours across joined navigation pieces

Compilation can now join pieces that all declare `preserveMovementBoundary`.
It assembles the outer boundaries separately from blocked contours. A cutout that
extends into a neighboring surface does not block a route that surface opens.
State contours retain their implicit outer-boundary intersections while excluding
neighboring pieces on other height planes. Mixed preserved/non-preserved pieces
remain an explicit error.

Focused regressions cover crossing outer contours, another surface opening a
cutout, and state changes restricted to their own height plane. The experimental
Nottingham output under `work/map-compile/nottingham-state-navigation-native-preserved`
constructs in Rust. Its courtyard envelope and both changing-state coverages match,
but permanent free coverage differs by about 158.446 square units in both states.
The ground transition's full free coverage also differs by one square unit, even
though its changing obstacle coverage matches. These are broader comparisons than
the changing-contour check alone. Neither difference is accepted as parity, and
the committed authoring catalogs do not yet enable this experimental mode.

Reviewed navigation recovery now accepts an explicit `preserveMovementBoundary`
setting alongside authored vertices. Clearance recovery subtracts exclusion
contours geometrically instead of assuming they are interior holes, and removes
floating contact slivers below its geometric tolerance while retaining subpixel
openings. The full-contour Nottingham draft now produces valid asset definitions
under `work/map-compile/nottingham-state-contours-recovery`.

That draft initially failed compilation: unioning nearly coincident cutout edges near
projected coordinates (696.9921, 1191.7559) fails to close a polygon ring. The
compiler reports the error; no rounded replacement or incomplete export is emitted.
The independent-contour change below resolves this compilation failure.

### Independent exclusion contours

Unlabelled exclusions now remain independent through normalization. Testing
containment against any of them already represents their union; merging them
first introduced unnecessary fractional vertices and could fail on nearly
coincident edges. Explicitly labelled fragments still reassemble as one contour.
Joined regions also preserve each cutout against free coverage from other pieces
without subtracting its own free coverage again.

The experimental recipe `nottingham-state-contours.json` retains complete permanent
contours and enables boundary preservation. It replaces the navigation recipe in
the courtyard recovery command; the other ownership, mask, ground-receiver and
physical-projection recipes remain the same. Both baseline and moved facade
exports compile and apply/reset all nine transitions in Rust. Both source movement
changes now have matching envelopes and changing-obstacle coverage in both states.
Permanent free coverage still differs by about 12.898 square units in the courtyard
and one square unit on the ground. Neither difference is accepted as full parity.
The six-receiver scan still passes all 2,170,166 Rust queries with zero height,
material or receiving-coverage differences; all twenty compiled masks are unchanged.

The default nine-map compilation regression also passes, and all forty previously
verified static ground areas retain zero coverage difference. These checks do not
cover the three missing transitions, complete traversal, visual data or publication.

### Exact Nottingham ground coverage and redundant fragments

The combined Nottingham recovery now uses `--precise-ground-ownership` with
`--preserve-ground-boundaries`, `--require-movement-coverage`, and the
`nottingham-state-contours.json` navigation recipe. This removes the one-square-unit
ground gap: both initial and applied full walkable coverage match exactly.
Candidates and diagnostics are under `work/map-compile/nottingham-precise-contours-recovery`
and `work/map-compile/nottingham-precise-contours-native`.

The compiler also discards a fractional cutout already entirely covered by a
complete integer exclusion in the same contour group, before rounding can expand
it beyond that exclusion. A regression covers both input orders and verifies that
a fragment extending outside the exclusion is retained. This removes a spurious
corner near the courtyard's southern obstacle. Its remaining permanent-coverage
difference is about 11.614 square units in each state, concentrated at stair seams.
Both movement envelopes and changing-state contours still match exactly; neither
this improvement nor the exact ground region certifies full map parity.
The baseline and moved facade still apply/reset all nine compiled transitions.
The six-receiver Rust scan passes 2,170,166 queries with zero differences, all
twenty mask records are unchanged, and the nine-map regression retains exact
coverage for its forty checked static ground regions.

### Complete Nottingham stateful walkable coverage

Reviewed navigation boundaries already carry explicit blocked contours. Their
one-time clearance recovery now clears derived solid slices across the entire
authored outer boundary, including those exclusions. The explicit contours remain
blocked; duplicate rounded collision slices no longer add false stair seams.
Physical volumes and their collision away from the reviewed surfaces remain intact.

The combined precise draft now has zero full walkable-coverage difference for both
Nottingham movement changes, initially and after application. Both outer envelopes
and changing-obstacle coverages also match exactly. The independent twenty-unit
stair move retains all four physical shapes and flags and separates their shared
receiving region from the courtyard. Baseline, moved facade and moved stairs all
construct in Rust and apply/reset all nine compiled transitions. Their native
area counts are 95, 95 and 96 respectively; each retains 172 doors and 38 jump pairs.
The six-receiver Rust scan passes all 2,170,166 queries with zero height, material
or receiving-coverage differences, and all twenty compiled mask records are unchanged.

Re-running Croisement03 recovery with this change retains exact permanent coverage
for all eleven movement regions and exact initial/applied changing coverage for all
nine transitions. Nottingham still has 507 unrecovered masks, 23 shadow regions,
18 sound sources, and unverified actor traversal and visual-state behavior. These
drafts remain unpublished and are not certified at full map parity.

### Complete crossing-map movement state ownership

`croisement01-state-assembly.json` groups the two physical parts of its missing
change, and `croisement02-state-assembly.json` groups the four parts of its missing
change. Each set has one shared state controller and no other controlling change.
Staging retains exact mesh content, transforms and collision shapes and reopens
the pinned scenes. The committed `croisement01-navigation-ownership.json` and
`croisement02-navigation-ownership.json` retain the other asset-local navigation
state owners.

Recovery uses each new staged scene, its ownership and ground-receiver catalogs,
and `--preserve-ground-boundaries --precise-ground-ownership --require-movement-coverage`.
Croisement01 also retains its existing mask catalog; Croisement02 has no reviewed
mask catalog yet. Staged libraries, candidates and native diagnostics use the
`work/map-compile/croisement01-state-assembly-*` and
`work/map-compile/croisement02-state-assembly-*` prefixes.

All six Croisement01 transitions and all nine Croisement02 transitions now recover.
For all fifteen, the baseline envelopes, permanent coverage and initial/applied
full walkable coverage match exactly. Both maps construct in Rust and apply/reset
every transition in baseline and one-pixel assembly-move variants. The moved parts
retain exact ordered physical vertices and flags, and their waypoints follow them.
The native exports contain respectively 8/5 movement areas, 92/154 sight obstacles,
16/5 doors and 13/4 jump pairs.

Comparison with the previously scanned drafts confirms unchanged ordered physical
sight geometry, receiving planes, materials, masks, receiving layers and initial
receiving-region coverage for all 22/12 receivers. This is a structural regression
check, not a new native query scan. Croisement01 still has 102 pending masks;
Croisement02 has 142 pending masks and five sound sources. Visual state masks,
complete actor traversal and publication remain outstanding.

### York market movement-state assembly

`york-state-assembly.json` stages the 47 rendered market parts controlled by one
shared state change. Static structural parts remain in separate remainder assets.
`york-state-ownership.json` moves the two additional non-rendering volumes to the
same assembly while retaining the other explicit volume, door and interior owners.
The stage preserves model content and transforms and reopens its pinned scene.

`york-state-ground-receivers.json` binds seventeen physical receivers to their
shared ground regions, including twelve on the stateful market region. Their
physical heights and materials remain independent of navigation. Without these
bindings, separate receiving footprints incorrectly replaced large portions of
the ground movement envelope. Recovery also uses the unchanged York mask catalog,
`--preserve-ground-boundaries --precise-ground-ownership --require-movement-coverage`.
Use `work/map-compile/york-state-assembly-stage` for the library/scene and
`work/map-compile/york-state-ground-recovery` for the combined candidate output.

The baseline market envelope, permanent coverage and full initial/applied walkable
coverage now match exactly. All eight ordinary ground regions retain exact
envelopes and permanent coverage; the two lift regions are outside that comparison.
The baseline and one-pixel assembly move preserve all 39 initial and 10 applied
physical shapes and flags, including the non-rendering volumes. The waypoint moves
with the assembly. Both exports construct 161 movement areas, 1,180 sight obstacles,
254 doors and 72 jump pairs in Rust and apply/reset all six compiled transitions
(one movement change and five door-only changes).

Native receiving checks cover 168 windows around every receiver vertex and reviewed
navigation anchor: 433,568 queries have zero height, material or coverage differences.
This is targeted sampling, not a full-footprint scan. All twenty existing mask
contents remain unchanged, with layer indices rebuilt for the new topology; complete
mask-layer/visual fidelity remains unverified. The candidate still has 808 pending
masks, sixteen shadow regions and eight sound sources. Actor traversal, visual state
assets and publication remain outstanding. Completing movement-state ownership
does not certify any map at full gameplay parity.

### Complete crossing-map environmental sound definitions

`croisement02-ambient-sounds.json` authors five independent acoustic regions;
the sixth emitter already belongs to a visual asset. The expanded
`croisement03-ambient-sounds.json` authors all six regions. Each recipe pins the
source document for one-time authoring. Generated definitions store emitter
geometry in asset-local coordinates, together with sample IDs, timing, falloff,
volume, acoustic altitude and ambience filters. Compilation needs only the
placed assets and scene, not the extraction source.

The combined stages in `work/map-compile/crossing-sound-library/{croisement02,croisement03}`
extend the latest movement-assembly scenes and reopen their pinned documents.
Recovery retains the movement coverage gate, ground receivers and, for
Croisement03, the terrace navigation joins, transition planes, projections and
mask definitions. Both reports now have zero pending sound sources; their
142/130 pending masks remain unchanged.

Both baselines reproduce all six source emitters exactly and retain identical
non-sound geometry to their preceding movement drafts. Eleven additional
exports move each newly authored region independently by 50 pixels: the selected
emitter follows its placement and all non-sound compiled geometry stays identical.
All thirteen exports load and construct in Rust, including sound sample selection,
emitter handles, spatial geometry and delay settings. Every export also applies
and resets all nine compiled transitions. These diagnostics are in
`work/map-compile/crossing-sound-native`; they do not verify audible playback,
complete visual state fidelity or a published ZIP round trip. Neither map is
certified or published at full parity.

### Derby, Leicester and Sherwood environmental regions

The expanded Derby and Leicester ambient catalogs and new
`sherwood-ambient-sounds.json` define respectively nine, fourteen and five
independently placed sound regions. These emitters have no containing visual
asset; existing uniquely owned and global emitters retain their previous owners.
The regions preserve full polylines, including points beyond the map boundary,
and retain sample, delay, falloff, volume, altitude and ambience settings.

Combined candidates in `work/map-compile/town-forest-sound-recovery` recover all
12/24/8 emitters with zero pending sound ownership. Their libraries extend the
Derby canopy-pole stage, Leicester projection/material stage and Sherwood central
treehouse stage, retaining ground receiver definitions and Sherwood navigation
joins/projections. The recovered scenes pass the movement coverage gate. Pending
masks remain 227/449/166 and shadow regions 24/23/1, respectively.

The native construction diagnostic accepts an optional `ambience` value on each
manifest result, applied only to its test mission. This permits checking sound
filtering under each ambience bit without adding mission settings to map exports.

All 44 baseline emitter records match exactly. Moving each of the 28 new regions
by 50 pixels moves only that emitter; non-sound geometry stays identical both
across these variants and against the preceding map drafts. The 31 exported
descriptors in `work/map-compile/town-forest-sound-native` pass 52 Rust construction
cases: baseline maps under all eight ambience bits, plus the moved-region exports.
These check required sample selection, emitter handles, shape and delay settings.
All compiled transitions also apply/reset successfully (Derby two, Leicester
six, Sherwood zero). Audible playback, publication and full ZIP round-trip
parity remain unverified.

### Nottingham environmental regions

`nottingham-ambient-sounds.json` now defines twenty independent sound regions,
including the two previously authored northern boundary lines. The eighteen new
regions have no containing visual asset; the other four emitters retain their
existing asset owners. Several independent emitters use the same sample ID, so
comparisons retain record multiplicity and full geometry rather than treating
sample IDs as unique emitter identifiers.

`work/map-compile/nottingham-sound-library/nottingham` extends the current
movement-assembly scene, retaining its existing sound pins. Combined recovery
uses the state ownership, state mask, ground receiver, state projection and
complete state contour catalogs with precise ground ownership and the movement
coverage gate. The report in `work/map-compile/nottingham-sound-recovery/nottingham`
has zero pending sound sources; 507 masks and 23 shadow regions remain pending.
All 24 baseline emitter records match exactly, and non-sound geometry is identical
to the preceding precise-contour draft.

The native batch in `work/map-compile/nottingham-sound-native` contains 22 exports:
the baseline, twenty independent 50-pixel region moves, and a duplicated emitter
whose sample is also used by other regions. All variants preserve non-sound
geometry; duplication adds exactly one correctly placed emitter. Twenty-nine Rust
construction cases pass, including the baseline under all eight ambience bits.
Every case also applies and resets all nine compiled transitions. The geometry
remains 95 movement areas, 659 sight obstacles, 172 doors and 38 jump pairs.
Audible playback, visual completeness, publication and ZIP round trips are still
outstanding; this does not certify full map parity.

### Lincoln and York independent environmental regions

`lincoln-ambient-sounds.json` defines seven independent regions and
`york-ambient-sounds.json` defines six. These emitters have no containing visual
asset. The remaining four ambiguous emitters have not been assigned arbitrarily:
Lincoln sources 1/13 overlap the west tower/hillside and great hall/plateau;
York sources 12/20 overlap a house/raised terrain and market frontage/shared
volume/raised terrain. They still require explicit asset ownership.

The combined stages in `work/map-compile/lincoln-york-sound-library` retain
Lincoln's door-turret volume scene and York's market movement assembly scene.
Recovery uses their mask and ground-receiver definitions; York also retains
its state ownership catalog and precise ground ownership. Both pass the
movement coverage gate. Reports in `work/map-compile/lincoln-york-sound-recovery`
now recover 12/14 Lincoln and 21/23 York emitters, with exactly those four
ownership gaps. Pending masks remain 424/808 and shadow regions 25/16.

All recovered emitter records match exactly, including repeated sample IDs.
Thirteen independent 50-pixel moves affect only the selected sound region.
Non-sound geometry also matches the preceding Lincoln ground-receiver and York
state-ground drafts exactly. Fifteen exports in
`work/map-compile/lincoln-york-sound-native` pass 29 native construction cases,
including both baselines under all eight ambience bits. These are partial
sound-definition checks, not audible playback or full-map parity certification.
All nine Lincoln and six York compiled transitions also apply/reset successfully
in every case. Publication and complete ZIP round trips remain outstanding.

### Explicit building-owned environmental sounds

Ownership catalogs now accept `sound_sources` declarations with a source record,
asset, node and review reason. Recovery validates the complete pinned sound record,
requires exactly one placed frame, and rejects duplicate declarations or conflicts
with independently authored sound assets. Only the localized emitter definition
is written into the asset gameplay packet; compilation does not read the catalog
or extraction source.

`lincoln.json` attaches the tower and hall emitters to those building assemblies.
`york-state-ownership.json` attaches the two remaining emitters to the jettied house
and visible market-frontage house. These are explicit authoring decisions: their
underlying terrain and shared volumes retain separate ownership. Combined reports
in `work/map-compile/declared-sound-recovery` now recover all 14 Lincoln and 23 York
emitters with zero pending sound sources. Baseline non-sound geometry is unchanged.

Six diagnostics in `work/map-compile/declared-sound-native` cover both complete
baselines and four acoustic probes using building frames translated by 50 pixels.
All emitter values match, with only the selected emitter moving. Twenty Rust
construction cases pass, including both baselines under all eight ambience bits.
The moved acoustic probes retain baseline physical geometry and do not establish
full-building relocation parity. Moving the entire Lincoln west tower by 50 pixels
detaches its elevated door from its walkable landing; full compilation correctly
rejects the missing exterior surface instead of inventing a connection.

The ownership tests cover changed source records, duplicate claims, global sources,
missing/ambiguous frames and empty review reasons. Complete visual assets,
valid relocated traversal assemblies, publication, audible playback and ZIP round
trips remain outstanding.

The consolidated `work/map-compile/complete-sound-definition-audit.json` compares
the complete emitter-record multisets for all nine source-backed staged maps:
Croisement01/02/03 2/6/6, Derby 12, Leicester 24, Lincoln 14, Nottingham 24,
Sherwood 8 and York 23. All 119 records match exactly, including duplicate sample
IDs. This closes staged sound-record coverage, not playback, publication or
Wychford authoring. Other map compilation categories remain incomplete.

### Independently authored environmental lighting fields

`author-light-region-assets.ts` builds invisible light-region assets from a
hash-pinned recipe. Contours and any receiving anchors are stored in asset-local
coordinates with their ambience filter. Their runtime definitions contain no
source-layer lookup. A `light_sources` ownership declaration pins the complete
source record and resolves exactly one placed asset frame during one-time
recovery; duplicate claims, changed records and missing/ambiguous frames fail.

Sherwood's western night field spans multiple structures and terrain with no
single containing visual asset. `sherwood-light-regions.json` authors it as an
independent region, and `sherwood-light-ownership.json` retains the existing
Sherwood ownership declarations while adding its explicit lighting owner.
Generate the asset with:

```sh
node pipeline/src/author-light-region-assets.ts \
  --source library/game-data/Data/Levels/Sherwood.rhp.json \
  --recipe refinement/catalogs/sherwood-light-regions.json \
  --map Sherwood --out work/map-compile/sherwood-light-stage
```

The staged scene extends the latest Sherwood sound library with the emitted
`light-region-assets.json` fragment and reopens its pinned assets. Recovery uses
the explicit light ownership catalog plus the existing ground receiver,
navigation join and projection definitions, with the movement coverage gate.
`work/map-compile/sherwood-light-recovery` now has zero pending shadow regions
and sound sources. Its 166 pending masks remain unfinished.

The baseline and a 50-pixel region move in `work/map-compile/sherwood-light-native`
reproduce the complete contour and ambience exactly. All non-light geometry and
metadata, including the eight sound emitters, are unchanged. Native light queries
pass for both exports under ambience bits 1, 2 and 4. Unit tests cover independent
movement/duplication, invisible geometry and rejected ownership declarations.
This verifies an environmental field, not baked image lighting, rendered visual
parity, publication or a complete ZIP round trip.

### Elevated lighting fields with incomplete physical coverage

Lighting fields retain a complete 2D contour on their resolved navigation layer;
the contour need not be physically supported at every point. Field recovery now
uses supported interior anchors when an elevated contour extends beyond physical
receiving coverage. It never invents a ground plane for that uncovered portion.
Strict per-plane recovery still rejects missing elevated geometry, and field
recovery fails if it cannot establish receiving anchors. Ownership checks retain
the full contour footprint so uncovered margins do not silently acquire an owner.

The geometry-only audit in `work/map-compile/light-field-audit.json` now preserves
147 of 148 complete contours across Derby (26), Leicester (30), Lincoln (30/31),
Nottingham (24) and York (37). Lincoln source 11 still fails with a degenerate
height plane. This audit does not assign owners or prove compiled receiving-layer
equivalence for every field; the earlier per-map pending reports remain in force.

`derby-light-regions.json` authors the previously rejected west-steps night field
(source 4). Its asset in `work/map-compile/derby-light-field-stage` compiles alongside
the existing Derby gameplay definitions. The complete contour and ambience match;
its source layer 1 resolves to rebuilt layer 23, whose receiving plane matches the
step plane. The diagnostic in `work/map-compile/derby-light-field-native` contains
three light regions and preserves all non-light geometry and metadata. Native
queries pass under ambience bits 1, 2 and 4. This is one additional compiled field,
not a fully recovered or published Derby map.

### Receiving planes survive tiny light-field intersections

Field recovery now carries each physical receiving plane through clipping instead
of reconstructing it from a clipped triangle. Very small valid triangles can fall
below the plane solver's nondegeneracy threshold even when their original support
has a well-defined plane. Their coverage and receiving anchors are retained;
the fix does not discard pieces or simplify the light contour.

The regression test includes a tiny receiving triangle whose vertices cannot
independently define a stable plane. Its field still preserves the full contour
and the correct elevated anchor. The geometry-only audit now succeeds for all
148 contours across Derby, Leicester, Lincoln, Nottingham and York, including
Lincoln source 11. This supersedes the one remaining geometry error above;
ownership and compiled receiving-layer validation are still incomplete overall.

`lincoln-light-regions.json` authors that previously failing elevated field into
`work/map-compile/lincoln-light-field-stage`. It compiles alongside the complete
Lincoln sound definitions into `work/map-compile/lincoln-light-field-native`, with
seven light contours and unchanged non-light data. The added contour and ambience
match exactly, with source layer 2 resolved to rebuilt layer 10. Native light
queries pass under ambience bits 1, 2 and 4. Publication and full map/ZIP parity
remain outstanding.

### Complete Derby light-region definitions

The combined Derby stage now retains eight building-owned regions and eighteen
independent environmental fields. `derby-light-regions.json` contains the eighteen
field recipes; `derby-light-ownership.json` retains Derby's existing ownership
definitions and pins their explicit field owners. The reopened scene in
`work/map-compile/derby-light-stage` extends the complete sound stage. Recovery in
`work/map-compile/derby-light-recovery` has zero pending shadow regions or sound
sources; 227 masks and visual patch definitions remain unfinished.

Combining all regions exposed two anchor issues. Recovery now excludes permanent
movement obstacles when choosing field anchors. Compilation also retains fractional
positions for light layer-selection anchors: rounding a valid interior anchor can
move it outside a narrow contour or receiving surface. The exported contour vertices
remain quantized. Regression tests cover both cases, and geometry recovery still
preserves all 148 contours in the five-map audit.

The export in `work/map-compile/derby-complete-light-native` contains all 26 source
outlines and ambience filters with no extra outline. These produce 30 runtime
regions: source region 16 spans five rebuilt receiving layers. The per-source
layer mapping is recorded in `source-light-mapping.json`. All non-light geometry
and metadata match the prior sound-complete draft. Native construction and light
queries pass for all 30 runtime contours under ambience bits 1, 2 and 4.

This establishes complete staged contour coverage and successful native queries
on the emitted layers. It does not yet establish source-versus-compiled lighting
equivalence for every actor position, rendered appearance, publication or ZIP
round-trip parity.

### Derby source-defined lighting query comparison

The native diagnostic
`recovered_lights_match_source_queries_on_shared_walkable_coverage` compares
source contour membership against the compiled engine's actual light queries.
Its `ROBIN_LIGHT_COMPARISON` manifest specifies source/compiled layer pairs and
their common navigation coverage, excluding permanent obstacles. It samples an
integer/half-pixel grid under ambience bits 1, 2 and 4 and writes a query report.

Derby's `work/map-compile/derby-complete-light-native/light-query-comparison.json`
has thirty nonempty query windows covering the full bounds of the relevant source
and compiled contours, expanded by two pixels. The windows include potentially
unwanted light from other source layers sharing a rebuilt layer; candidate windows
without common walkable coverage are recorded separately. All 4,177,323 query
evaluations match. Counts include repeated positions across windows and ambiences.
Changing one exported field's ambience in a separate negative-control descriptor
produces 305,058 differences, confirming that the comparison detects a real error.

This adds positional evidence beyond matching contour records. It uses the known
light-layer mappings and common navigation domains, so it does not certify missing
walkable coverage, unmapped layer pairs, altered placements, mission transitions
or rendered appearance. Full-map publication and ZIP round trips remain unfinished.

### Complete Nottingham light-region definitions

`nottingham-light-regions.json` authors nineteen independent environmental fields;
five additional regions remain building-owned. `nottingham-light-ownership.json`
retains the movement assembly and existing ownership definitions while adding the
explicit field owners. The reopened scene in `work/map-compile/nottingham-light-stage`
extends the complete sound library and retains its navigation contours, physical
projection receivers, ground receivers and reviewed mask definitions.

The combined recovery in `work/map-compile/nottingham-light-recovery` has zero
pending shadow regions or sound sources, with the movement coverage gate passing.
All 24 light contours and ambience filters compile exactly into 24 runtime regions
in `work/map-compile/nottingham-complete-light-native`. Non-light data matches the
preceding sound-complete draft, including all sound emitters. The engine constructs
the map and applies/resets all nine compiled transitions.

The source-query comparison covers 24 nonempty windows on known layer pairs under
ambience bits 1, 2 and 4. All 6,314,187 integer/half-pixel query evaluations match
the source contours. Candidate windows without shared walkable coverage are
recorded separately. As with Derby, this verifies mapped common navigation domains,
not missing geometry, unmapped layer pairs, altered placements or rendered appearance.
Nottingham still has 507 pending masks and incomplete visual patch definitions;
publication and complete ZIP round trips remain outstanding.

### Complete York light-region definitions

`york-light-regions.json` authors sixteen independent environmental fields;
twenty-one additional regions remain on existing assets. `york-light-ownership.json`
retains the market movement assembly and complete sound ownership while adding
the explicit field owners. The reopened scene in `work/map-compile/york-light-stage`
extends the complete sound library and retains its reviewed ground receivers
and mask definitions.

The combined recovery in `work/map-compile/york-light-recovery` has zero pending
shadow regions or sound sources, with the movement coverage gate passing. All
37 source light contours and ambience filters compile exactly into 37 runtime
regions in `work/map-compile/york-complete-light-native`. Non-light data matches
the preceding sound-complete draft. The Rust engine constructs 161 areas,
1,180 sight obstacles, 254 doors and 72 jump pairs, and applies/resets all six
compiled transitions.

The source-query comparison covers 37 nonempty windows on known layer pairs
under ambience bits 1, 2 and 4. All 10,299,072 integer/half-pixel query evaluations
match the source contours. Seven candidate windows without shared walkable
coverage are recorded separately. This verifies mapped common navigation domains,
not missing geometry, unmapped layer pairs, altered placements or rendered
appearance. York still has 808 pending masks and incomplete visual patch
definitions; publication and complete ZIP round trips remain outstanding.

### Leicester light definitions and detected layer leakage

`leicester-light-regions.json` authors fifteen independent environmental fields;
fifteen additional regions recover onto existing assets. The explicit field
owners are in `leicester-light-ownership.json`. The combined scene in
`work/map-compile/leicester-light-stage` extends the sound-complete draft and
retains its ground receivers and reviewed mask definitions. Recovery has zero
pending light or sound records and passes the movement coverage gate.

All 30 source contours and ambience filters compile into 34 runtime regions in
`work/map-compile/leicester-complete-light-native`; some contours receive on
multiple rebuilt layers. Non-light data matches the preceding sound-complete
draft. Rust constructs 55 areas, 444 sight obstacles, 105 doors and 23 jump pairs,
and applies/resets all six compiled transitions.

**Initial lighting query parity failed (fixed below).** The source-query comparison checks 39 nonempty
windows on known layer pairs under ambience bits 1, 2 and 4. Of 4,283,946
integer/half-pixel evaluations, 1,436 differ. For example, at `(279, 920)` under
ambience 4, source layer 4 is unshadowed but compiled layer 7 is shadowed. The
compiler groups equal-height planes onto the same layer: this combines receiving
regions from source layers 2 and 4, allowing a contour belonging to one region
to shadow its neighbour. The contour and filter inventory is therefore complete,
but layer allocation must preserve independently authored light receivers before
this draft can pass. The comparison manifest and failure report retain the
reproduction; empty shared-coverage windows are recorded separately.

Leicester also retains 449 pending masks and incomplete visual patch definitions.
These staged definitions are not published or certified, and complete ZIP round
trips remain outstanding.

### Separate light receivers on otherwise shared planes

Flat light recovery now retains asset-local receiving anchors, just as multi-plane
recovery does. A height plane alone cannot distinguish unrelated receiving regions.
All 148 source light contours across Derby, Leicester, Lincoln, Nottingham and York
still recover exactly with these anchors. Earlier staged packets need regeneration
to acquire anchors for their flat fields.

After assembling navigation regions, compilation separates regions that would
otherwise receive another region's anchored light on a shared layer. Compatible
regions keep sharing layers. Full light contours remain intact, traversal keeps
the final reserved layer, and sector, door, projection and transition references
are assigned after the new ordering. Unanchored planar fields can cover multiple
resulting layers. Compiler tests cover independent coplanar receivers, a field
covering both, and moving the asset.

The refreshed Leicester draft in `work/map-compile/leicester-separated-light-native`
retains all 30 contours and filters as 34 runtime regions. Its 37 nonempty comparison
windows pass all 4,277,196 Rust lighting query evaluations under ambience bits 1,
2 and 4. Restoring the leaking contour to the wrong layer in a separate negative
control causes 704 differences, including the original `(279, 920)` failure.
These totals count window evaluations, not unique positions; changing the layer
partition changes overlapping windows. Coverage remains limited to mapped common
walkable domains, not unrestricted gameplay or rendered appearance.

Motion contours and obstacles are unchanged. Non-light records compare equal
after resolving sector/layer references to their receiving geometry and sorting
generated projection records; mask payloads compare without their rebuilt layer
numbers. Rust still constructs 55 areas, 444 sight obstacles, 105 doors and 23 jump
pairs and applies/resets all six transitions. This does not establish mask-layer
or visual parity, publication, or complete ZIP round trips.

### Complete Lincoln lighting and receiving-area anchor recovery

`lincoln-light-regions.json` now authors eleven independent environmental fields.
`lincoln-light-ownership.json` preserves Lincoln's existing ownership definitions
and explicitly assigns two elevated fields to the great hall and keep, where
their footprints also overlap the underlying plateau. Eighteen other fields
recover onto existing assets, including the west slate tower field previously
tested as an independent prototype. The combined stage extends the sound-complete
library and retains its ground receivers and reviewed masks.

This exposed an anchor-recovery error: an overlapping sloped projection belonging
to another motion area supplied a terrace anchor's height. Offline light recovery
now uses each source motion area's receiving-sector identity when selecting its
anchor support. The emitted assets retain local coordinates, not source sector
indices. A regression covers overlapping higher footprints, and all 148 town
contours still recover with receiving identities enabled.

`work/map-compile/lincoln-light-recovery` has zero pending light or sound records
and passes the movement coverage gate. Its 31 contours and ambience filters
compile into 33 runtime regions in `work/map-compile/lincoln-complete-light-native`.
Non-light records compare equal after resolving sector/layer references to
receiving geometry and sorting generated projection records; mask payloads
compare without their rebuilt layer numbers.

All 5,749,329 Rust lighting query evaluations match across 36 nonempty windows
on known layer pairs under ambience bits 1, 2 and 4. Empty common-coverage windows
are recorded separately. Rust constructs 62 areas, 541 sight obstacles, 89 doors
and 10 jump pairs and applies/resets all nine compiled transitions. This verifies
mapped shared walkable coverage, not missing geometry, unmapped layers, changed
placements or rendered appearance. Lincoln still has 424 pending masks and
incomplete visual patches; publication and complete ZIP round trips remain open.

### Derby main-hall static masks

`derby-masks.json` now includes the main hall's west/east gallery cover records
134 and 135. A fresh audit of the pinned hall mesh supports every covered pixel
of both records; neither belongs to a state patch or has obstacle links. Their
receiving anchors select the adjacent 465.001-unit gallery platform. The east
mask's character threshold continues that plane, and projectile thresholds retain
the world XY datum.

`work/map-compile/derby-main-hall-mask-recovery` recovers eleven mask records,
reducing Derby's pending count from 227 to 225. In the baseline export, the two
added records reproduce all 4,692 and 23,708 covered pixels respectively, plus
their character/projectile polylines and application flags. Rust constructs the
combined export's 60 areas, 337 sight obstacles, 70 doors and two jump pairs and
applies/resets its two transitions.

The initial whole-hall relocation test failed (fixed below): moving the hall by +1 X failed because
its `light-19` anchor moves over the neighbouring sloped gallery without acquiring
the slope's changed height (authored 495.4359517424076 versus receiving
495.60863123076155). This failure is separate from the mask pixel comparison and
must be resolved before claiming movable-hall parity. The baseline evidence does
not certify mask-layer semantics, visual rendering, publication or ZIP round trips.

### Finite light receiving segments for sloped attachments

Asset light definitions can now include local `receiverSegments`. After placement,
each finite segment must intersect exactly one receiving navigation sector, inside
the light contour. Segments transform with their owning part. Missing, ambiguous,
degenerate or coplanar attachments fail explicitly; the compiler does not perform
an unrestricted nearest-floor search. Layer-conflict allocation also considers
these attachments before assigning fresh sector indices.

Offline recovery emits segments for sloped receivers, bounded by the receiving
plane's heights over the source motion area's footprint. Flat receivers retain
exact point anchors. Only local endpoint coordinates enter the asset definition.

The refreshed Derby draft in `work/map-compile/derby-segment-light-recovery` now
allows the previously failing +1 X main-hall move. Both new masks preserve every
covered pixel and shift their boundary rules exactly; the gallery light retains
its complete shifted contour and ambience filter on the traversal layer. Baseline
compiled geometry matches the preceding mask draft except light-record ordering.
Rust constructs both baseline and moved exports and applies/resets both transitions
in each. Synthetic tests reject ambiguous/missing receivers and verify slope
intersection and finite search bounds. This is evidence for that placement change,
not arbitrary relocation, full rendering parity, publication or ZIP round trips.

### Editor loading of generated environmental assets

The earlier light/sound authoring tools emitted a bare empty node. That worked in
compiler diagnostics, which read metadata directly, but failed the editor's
standalone asset validation. Both generators now use a common model writer that
emits the Z-up `map` wrapper, one identity asset group with its asset ID, and the
declared scenery/gameplay-only frame. The asset remains invisible and contains no
placeholder mesh.

An integration test loads actual generated light and sound GLBs through
`prepareProjectionAsset` without mocking GLB parsing, inserts them, serializes and
reopens their editor placements, and reloads their pinned descriptors. It checks
that their complete gameplay definitions survive, including a sloped light's
finite receiving segment. The authoring/compiler tests continue to verify sound
records and light contours after placement. This closes an editor insertion/reload
gap, not map publication or in-game ZIP round-trip parity. Previously staged field
GLBs and their saved model hashes must be regenerated together before publication.

### Refreshed environmental field model libraries

`work/map-compile/editor-field-model-library/<map>` now contains isolated refreshed
libraries for all ten editor scenes. The refresh reads existing asset definitions,
regenerates only their invisible field models with the corrected hierarchy, and
updates model hashes in the saved scene. Descriptor bytes, gameplay definitions,
all placements and other scene content remain unchanged. Visual asset files remain
linked to the preceding staged libraries; these are local working libraries, not
published self-contained packages.

All 152 regenerated fields pass the actual editor asset loader with pinned model
and descriptor hashes: Derby 27, Leicester 29, Lincoln 18, Nottingham 39, York 22,
Sherwood 6, Croisement02 5 and Croisement03 6. Croisement01 and Wychford have no
standalone fields to refresh. All ten saved scenes reopen with equivalent content;
the comparison normalizes only absent versus empty resource lists. A fresh palette
index is written for each scene's assets. The report is
`work/map-compile/editor-field-model-library/refresh-report.json` and the local
reproduction script is `work/map-compile/refresh-editor-field-models.mjs`.

This resolves the generated-model hierarchy and pinning problem for these combined
drafts. It does not publish the remaining recovered gameplay packets, refresh older
flat-light definitions with new attachments, remove pending masks or visual states,
or establish an in-game ZIP round trip.

### Reopened scenes compile from embedded asset definitions

`work/map-compile/embedded-gameplay-library/<map>` now embeds recovered gameplay in
the actual asset descriptors for nine source-backed maps: Derby 69, Leicester 100,
Lincoln 118, Nottingham 165, York 271, Sherwood 88, Croisement01 61, Croisement02 150
and Croisement03 99. Scene descriptor hashes and palette entries are updated together.
Placements and other scene content remain unchanged. The asset models and resources
remain linked to the preceding staged libraries; this is not a distributable bundle.

Each scene is serialized, reopened and compiled using only its pinned descriptors.
The compiler reads neither recovery packets nor source levels. A separate comparison
with the packet-injection workflow verifies identical static geometry under the
current compiler. Eight outputs also match their preceding native JSON snapshots
(with JSON's negative-zero normalization). York has an additional receiving layer
from the newer allocation logic; its refreshed comparison passes 10,298,961 lighting
query evaluations on mapped common walkable coverage, with zero differences.

Rust loads all nine exports in `work/map-compile/embedded-gameplay-native` and
applies/resets all 56 compiled transitions. Full-scene compilation still rejects
Derby, Leicester, Lincoln and Nottingham because of unsupported scene state
transitions. York, Sherwood and the three crossings pass that compiler gate, but
the recovery inventory still records 2,953 pending masks across the nine drafts
and incomplete visual patch definitions. Passing that gate does not certify parity.
Wychford's gameplay definitions remain separately unfinished.

Each staged library contains `gameplay-staging-report.json` with the pending
inventory, original review issues and full-scene compiler result. The local script
`work/map-compile/stage-derby-gameplay-definitions.mjs <map>` reproduces the staging
and comparisons. No pending items were waived and no map is marked published or
fully playable at parity. Visual/depth state integration, gameplay publication and
actual in-game ZIP round trips remain outstanding.

### Published keep view mask with stable appearance support

Leicester's great keep now includes `keep-west-view-occlusion`, recovered from
record 242 into its asset-local definition. All 1,814 pixels match exactly, using
92 triangles supported by the current pinned model. Those same triangles remain
identical across all four combinations of the keep's two appearance controls.
This static view-only mask has no character/projectile boundaries, obstacle links
or mask-state controls. Its receiving anchor is verified on ground.

The compiled mask retains exact coverage and rules after moving the keep one pixel
east. Baseline and moved exports pass native construction and mask verification;
both have 55 movement areas, 444 sight obstacles, 105 doors and 23 jump pairs.
Other baseline gameplay geometry and export warnings remain unchanged. Shared
descriptor references in Leicester and Wychford are repinned without changing
placements or mission content; the separate user test ZIP is unchanged.

This reduces the pending inventory from 2,953 to **2,952 masks** across the nine
recovered maps. Records 256 and 258 also reproduce their baseline coverage, but
remain deferred because their supporting triangles change with an appearance
state. Neither this batch nor native construction certifies full map parity.
The recipe is `refinement/catalogs/leicester-masks.json`; recovery, appearance
proofs and transaction backups are under
`work/map-compile/keep-static-view-publication` and
`work/map-compile/keep-static-view-state-proof.json`. The shared-reference repair
backup is `work/map-compile/keep-static-view-scene-pin-repair`.

### Published cottage, watermill and west-wing static masks

Four more Leicester masks are authored and published from the current pinned
models: southeast cottage 263 (823 pixels), watermill 173/174 (908/18,755 pixels),
and west wing 239 (1,175 pixels). Together they add 21,661 exact pixels represented
by 1,391 asset-local triangles. The cottage and watermill have no appearance
controls; the west-wing mask has identical supporting triangles before and after
its appearance switch. West-wing candidates 240/241 remain deferred because their
supporting surfaces change when revealed.

The three projectile/view masks retain their world-XY projectile thresholds;
the west-wing mask is view-only. None adds character thresholds, obstacle links
or mask-state controls. Each receiving anchor is checked against ground geometry.
The first watermill anchor was too close to a receiving-layer edge and failed the
independent move check. Selecting a nearby interior ground point fixes that
binding without changing mask geometry. All four now retain exact raster coverage
and rules after their owning asset moves one pixel east. Baseline navigation,
sight geometry, doors, lighting, sound and export warnings remain unchanged.

Only the four mask definitions are added. Five shared references are repinned:
three in Leicester and two in Wychford. Other descriptor and scene content is
unchanged, and all ten saved scenes reopen with valid pins. The separate user ZIP
is untouched. This brings the unrecovered inventory to **2,948 masks**; complete
map parity remains unverified. Baseline and all three independently moved exports
pass native construction and mask verification. Fresh exports of all ten published
maps also construct successfully, and all 56 switches apply and reset correctly.
The fresh all-map descriptors are under
`work/map-compile/published-receiver-mask-native`.

The recipe is `refinement/catalogs/leicester-masks.json`. State support evidence
is `work/map-compile/leicester-static-mask-state-proof.json`; transaction snapshots
and four baseline/independently moved native fixtures are under
`work/map-compile/leicester-static-mask-publication`.

### Published southwest-bank and south-hall masks

Leicester's southwest edge bank now owns view mask 226 (22,302 pixels), and its
south hall owns projectile mask 208 (5,059 pixels). All 27,361 covered pixels
match exactly using 1,898 asset-local triangles from the pinned meshes. Neither
asset has appearance controls; both records have no obstacle or mask-state links.
The hall's projectile threshold retains the world XY datum. Receiving anchors
are on adjacent navigable ground and move with their owning assets.

Both masks retain exact coverage and rules after independent one-pixel eastward
moves. Other baseline gameplay geometry and compile warnings are unchanged.
Rust constructs the baseline and both moved exports: 55 movement areas, 444 sight
obstacles, 105 doors and 23 jump pairs in each. These checks do not establish
full traversal or visual parity. All ten library scenes reopen with valid pins;
four shared descriptor references changed in Leicester and Wychford, with no
placement changes. The user ZIPs remain unchanged.

Recipes are in `refinement/catalogs/leicester-masks.json`; recovery evidence is
`work/map-compile/next-static-mask-review.json`, and publication snapshots plus
native fixtures are under `work/map-compile/leicester-bank-hall-mask-publication`.
The southeast manor candidate remains deferred because its proposed receiving
anchor did not pass the elevation check; no definition was published for it.

### Published Lincoln cliff and cottage masks

Lincoln's southwest cliff lower ledges now own view masks 62 and 64 (2,230 and
3,295 pixels); the village west cottage owns projectile mask 92 (351 pixels).
All 5,876 covered pixels match exactly. Recovery produces 16,346 asset-local
triangles from the pinned meshes. Neither asset has appearance controls, and
these masks have no obstacle links or mask-state references. Ground receiving
anchors have a four-unit interior margin at the reviewed placement; projectile
thresholds retain the world XY datum.

Baseline and independent one-pixel eastward moves retain exact coverage and
rules. Other baseline gameplay geometry and compile warnings are unchanged.
Rust constructs all three exports with 62 movement areas, 541 sight obstacles,
89 doors, 10 jump pairs and 132 elevation boundaries. All ten library scenes
reopen with valid pins. Only two Lincoln descriptor references change; placements
and user ZIPs remain unchanged. Full traversal, arbitrary rearrangements and
rendered parity remain unverified.

The recipes are in `refinement/catalogs/lincoln-masks.json`. Recovery evidence is
`work/map-compile/lincoln-next-static-mask-review.json`; publication snapshots and
native fixtures are under `work/map-compile/lincoln-static-mask-publication`.
The bailey cottage candidate 169 remains unpublished: mesh coverage passes, but
the proposed receiving anchor is too far away to accept without further review.

### Published York house projectile masks and relocation gap

Eight static projectile masks are now asset-local: castle west-lane front/rear
houses 569/574, east-quay front house 525, north-river-lane northern/rear/stone/low
houses 560/557/548/553, and riverside storehouse 2. Their 6,765 covered pixels match
exactly, using 1,574 triangles from pinned meshes without appearance controls.
They have no character boundaries, obstacle links or mask-state controls.
Projectile thresholds retain the world XY datum; receiving anchors lie on
nearby unblocked ground. Five central-city candidates lack a suitable nearby
ground anchor; stone-shop candidate 401 has a distant anchor. All remain deferred.

Each mask retains exact coverage and rules when its asset moves one pixel east.
Other baseline geometry and warnings are unchanged. Rust constructs the baseline
and all eight moved exports with 161 movement areas, 1,180 sight obstacles and
178 elevation boundaries. All ten saved scenes reopen, with only eight York
descriptor pins changed and no placement or user-ZIP changes.

The castle west-lane front-house move exposes a separate unresolved connection
gap: door projections drop from 254 to 252 and jump pairs from 72 to 70.
Jump sockets for pairs 70 and 71 require coincident join points within 0.0001
units; moving one house by one pixel disconnects both pairs. Other seven moves
retain the baseline counts. Mask translation passes, but this is explicitly
not proof of rearranged roof traversal parity. Flexible, bounded jump attachment
authoring and runtime traversal verification remain required; simply increasing
the matching tolerance could create ambiguous or invalid connections.

Recipes are in `refinement/catalogs/york-masks.json`; recovery evidence is
`work/map-compile/york-next-static-mask-review.json`. Publication snapshots and
the nine native fixtures are under `work/map-compile/york-static-mask-publication`.

### Lincoln gameplay after model republication

The revised Lincoln model publication omitted gameplay from 189 descriptors,
including terrain. Their asset origins changed, so copying the earlier local
coordinates would be incorrect. Fresh one-time recovery in the current placed
frames restores the ten lifts and publishes draft definitions for those 189
assets; the 18 existing independent light/sound definitions are retained.
The compiler continues to read only saved editor placements and asset metadata.
Models, placements and mission content are unchanged.

Recovery and rollback evidence is in
`work/map-compile/lincoln-republished-owned-recovery-20261004`. Native actor tests
pass all 16 stair routes and eight ladder/wall routes in both directions. The
current scene exports 50 navigation layers and ten lifts instead of two layers
and no lifts. These counts are coverage evidence, not full parity certification.
Recovery still reports one unowned building entry, 428 pending mask records,
seven appearance bindings and 12 patches. One tower light receiver is omitted
with a warning because its segment intersects multiple navigation surfaces.
Ground receiver and visual-state coverage still need review in the revised assets.

All ten current published scenes compile in `work/map-compile/stair-routing-9236w2`.
The complete native navigation audit passes 19 tests, including 288 stair routes,
84 ladder/wall routes and 6,220 receiving/ground crossings. It also retains the
constructed rotation sweep. Lincoln contributes 24 receiving crossings; no ground
receiver crossing was eligible there, which is a coverage gap rather than a pass.

Publication now checks that an existing gameplay definition is not silently
removed by a model replacement, both during preparation and application. Changed
asset frames require reconciliation; this guard does not validate the completeness
of a replacement definition. It does not block best-effort map exports.

The subsequent ground-receiver revision restores 19 reviewed receiving shapes.
Each current placed shape passes exact float32 geometry, flags, unique ownership
and persistent unblocked-anchor checks before its model pin is refreshed in
`refinement/catalogs/lincoln-ground-receivers.json`. Ground clearances are recovered
together across all assets: installing only the receiver-owning assets disconnects
six lifts because neighboring assets retain incompatible exclusions.

The published complete revision is in
`work/map-compile/lincoln-republished-ground-recovery-20261004`; it has 39 layers
and all ten lifts. Eight native tests pass, including all 24 Lincoln lift routes,
six ground crossings and 32 other receiving crossings. Reopening the published
document and compiling exclusively from its pinned assets reproduces the tested
geometry exactly. The tower light and visual-state/mask gaps above remain open.

### Bound sloped light attachments before neighboring floors

The Lincoln tower's missing light was an ambiguous attachment: its broad segment
intersected the intended stair at height 371.4914 and another floor at 340.001.
Recovery now restricts each finite segment to halfway between its intended
intersection and any other receiving floor above/below it. The bounds remain
inside the intended surface's elevation range. Only the resulting local segment
enters the asset definition; the compiler still resolves the placed geometry and
rejects missing or ambiguous receivers. Equal-height overlaps remain ambiguous.

The updated tower definition is published. Evidence is under
`work/map-compile/lincoln-bounded-light-recovery-20261004`. Baseline and a one-pixel
independent tower move export all 33 light regions and ten lifts without omitted
light warnings. Other baseline gameplay geometry is unchanged. Rust loads both
exports and verifies all 33 light contours/layers for ambience masks 1, 2 and 4.
Synthetic stacked-floor recovery also compiles after translation, vertical movement
and four rotations. This resolves the tower-light omission above, not the remaining
visual-state/mask coverage gaps or complete rendered lighting parity.

### Shared appearances on separately authored assets

`pipeline/src/author-linked-appearance.ts` authors an explicit appearance follower
for an existing control. The follower stores its own appearance ID, trigger geometry,
receiving anchors and matching contact in local coordinates. It does not copy the
controller's collision, movement, mask or door effects. Compilation assembles
matching placed contacts; moving one asset away leaves independent controls.
Conflicting control ownership and invalid coordinate conversions fail.

Three Lincoln relationships are now published: great hall and slate spire,
west slate tower and tower hall, and drawbridge mechanism and gatehouse appearance.
The six asset definitions and scene pins were updated together. Evidence and rollback
bytes are in `work/map-compile/lincoln-shared-appearances-l6H7Ry`. Native construction
passes with 62 movement areas, 546 sight obstacles, 86 doors and ten jump pairs;
all nine compiled transitions apply and reset successfully. Authoring tests cover
independent movement and conversion of rotated/elevated local control coordinates.

Four appearance bindings remain unresolved across the plateau, west terrace,
drawbridge and gatehouse. The bridge's physical raise/lower state, associated masks,
and rendered shared-state behavior still need verification. These published draft
links do not establish full appearance or map parity.

### Recover obstacle-only state controls

One-time recovery now handles changing sight/mouse volumes that have no movement
polygon or door changes. It requires a unique physical owner for every volume and
one owning asset for the complete change, retains separate persistent movement
exclusions, and reports missing ownership or unrecovered movement rather than
inventing a transition. These controls also participate in appearance and reviewed
mask binding. `sightTransitionRecovery` and `pending.sightTransitionBindings` expose
the additional recovery coverage.

Lincoln's drawbridge now has its raised obstacle state and associated appearance
control, separately from the existing six-door rights control. Its gatehouse
appearance is explicitly linked using local frames. Both asset definitions and
scene pins are published. Baseline and a jointly moved bridge/gatehouse compile
ten controls; native apply/reset passes all ten in both exports. Evidence and
rollback files are under `work/map-compile/lincoln-bridge-state-hWz7IQ`.

Two appearance bindings remain unresolved on the west plateau and terrace.
Animated lowering, mask coverage and rendered state changes remain unverified;
the new control is not evidence that the complete bridge sequence has parity.

A fresh library-wide authoring scan in
`work/map-compile/sight-state-library-recovery-20261004` produced 1,045 schema-valid
drafts across eight source-backed maps. Nottingham recovery stopped at a stale
declared door-owner frame before this new recovery stage; Wychford has no source
recovery by design. Those scan drafts were not published. This is an authoring
recipe gap, not a new runtime fallback or a claim of nine-map recovery success.

### Complete Lincoln appearance bindings

The remaining plateau and west-terrace appearances now share an explicitly
authored reveal control. Its local waypoint and activation contour were recovered
once; it carries no navigation, sight-volume or door-rights changes. Both asset
definitions and scene pins are published. The baseline now compiles in strict
mode with all appearance bindings resolved, 11 controls, ten lifts and 33 lights.
Native apply/reset passes all 11 controls for the baseline and jointly moved pair.

Evidence and rollback files are in `work/map-compile/lincoln-plateau-state-gaTztD`.
The one-pixel placement test retains all controls but uses best-effort export:
an independent light-region anchor has no receiving floor after the supporting
assets move. Its omission remains an explicit warning, not a passed lighting
placement check. Mask coverage, animation and rendered state parity remain open.

### Appearance bake cost and pixel fidelity

Async export now renders only full-frame tiles intersecting each appearance
region, then copies the region's pixels. The camera, depth scale, scene lighting
and tile grid remain unchanged. Rendering arbitrary cropped camera rectangles
changed edge rasterization, so the implementation deliberately retains the
original tile grid. Browser comparisons pass exact color/depth equality for
multi-tile crops and shadowed appearances under three sun directions; thirteen
appearance unit tests pass. GPU setup and shadow rendering are still repeated
per state, so large combination tables remain expensive. The full Lincoln archive
check is still outstanding; the small fixture is not full-map visual certification.

### Nottingham gateway ownership

The castle gateway's three portcullis lanes and door-rights control now belong
to the arch asset instead of the courtyard floor. Asset-local coordinates were
rebased without changing baseline exported door data; the arch appearance is
bound to that control. Both descriptors and the saved-scene pins are published.
The baseline and independent one-unit arch/floor moves pass native apply/reset
for all nine controls (`work/map-compile/nottingham-gate-control-OiiIqc`). The
control moves with the arch only. Portcullis animation and east-tower appearance
coverage remain unfinished.

The default Nottingham authoring recipe also names the current dormer-house
remainder instead of its pre-split asset. Its scan now completes with 165 valid
drafts; explicit gateway ownership reduces pending appearance bindings from two
to one (`work/map-compile/nottingham-gate-owner-recovery-20261004`). Other draft
geometry was not published. Older mask/light coverage totals are not superseded
by this scan, which did not supply the additional reviewed recovery recipes.

### Nottingham east-tower reveal

The remaining east-tower appearance now has an independent asset-local control.
It retains the activation point at its receiving height and carries no movement,
sight-obstacle or door-rights changes. Published baseline and one-unit tower-move
exports have ten controls with no missing appearance-binding warnings; both pass
native apply/reset. Reopening and compiling the published scene reproduces the
tested baseline exactly (`work/map-compile/nottingham-tower-state-KurPT1`). These
bindings complete the currently modeled Nottingham appearance controls, not the
unrecovered masks, animation or rendered-state coverage.

### Full Lincoln archive and current control audit

The full browser bake and Rust mod-loader check now pass for
`work/map-compile/lincoln-plateau-state-gaTztD/editor-lincoln-current.zip`.
The 152,115,399-byte ZIP includes the editable scene, color/depth/minimap and
three appearance regions with 48 states. Without a base datadir, Rust constructs
546 sight obstacles, 86 door projections and 69,920 grid blocks and validates
the appearance references against eleven controls. The descriptor contains ten
lifts and 33 light sectors, but zero explicit masks. This proves archive loading;
it does not prove mask coverage, animated effects or rendered gameplay parity.

A fresh ten-scene export snapshot in
`work/map-compile/published-controls-audit-20261004` passes native apply/reset for
all 60 compiled controls. Wychford still omits 25 doors, 15 masks, five jumps,
one lift and one control, and Derby/Leicester retain 98/203 unbound appearance-part warnings.
These omissions prevent a full-parity claim even though the exports load.

### Derby hall and upper-gatehouse reveals

Two additional appearance-only controls are published for the keep hall and
upper gatehouse. They retain asset-local activation geometry and receiving
height. The baseline and independent one-unit moves pass native apply/reset for
all four controls (`work/map-compile/derby-appearance-controls-lwHfaC`). The west
tower's existing mask control remains intact. The remaining 43 appearance-part
warnings belong to the east hall's unbound sight-state change; it requires its
physical state semantics rather than another appearance-only control.

### Derby and Leicester modeled appearance bindings

Derby's east hall now switches one initial sight volume to eight applied volumes
with its appearance. Its baseline navigation, doors and lifts remain unchanged;
native apply/reset passes all five controls for the baseline and moved hall.
The published scene matches `work/map-compile/derby-hall-sight-state-fWXBwB`.

Leicester now has twelve baseline controls, including separate keep, moat-tower
and west-wing sight changes. Three moat-reveal assets use explicit local contacts:
joint translation retains twelve controls, while detaching the church-side tower
produces thirteen independent controls. Native apply/reset passes all three
fixtures in `work/map-compile/leicester-appearance-controls-MJmVl8`, and reopening
the published scene reproduces its tested baseline. Baseline navigation/doors/
lifts are unchanged and no modeled appearance binding is missing in either map.

The Leicester joint-move fixture still warns about three independently placed
light-region receivers left behind by their supporting assets. These checks do
not certify light attachment after arbitrary moves, animated state sequences,
unrecovered masks or rendered gameplay. The previous ten-map audit remains a
historical snapshot rather than a claim that its omissions have all been fixed.

### Best-effort terrain reuse

Wychford's updated pinned assets compile successfully with two controls. Its
remaining omission breakdown is 25 doors, 15 masks, five jumps, one lift and one
control; many endpoints have no floor at their placed height. These counts must
not be described as 47 missing controls or assemblies.

Profiling showed terrain generation repeated during fallback retries. Compilation
now reuses terrain within one export, rebuilding it for every subsequent export.
The descriptor and all warnings are byte-identical before and after this change
(`work/map-compile/wychford-control-audit-JQmg56` and `wychford-control-audit-SHbyM9`).
Observed compile time dropped from 128.75 to 77.84 seconds, with profiling enabled
only for the baseline. This preserves exact geometry and does not repair the
reported placement gaps. Compiler and authored-terrain suites pass 120 tests.
The updated Wychford descriptor also passes native construction and apply/reset
for its two controls; that run took 176.82 seconds. Full traversal remains unverified.

### Spatially indexed native receiving boundaries

Native loading now indexes receiving polygons and boundary edges, narrowing
intersection and side-probe candidates before applying the existing exact tests.
Candidate order remains stable so equal-height receivers keep their precedence.
All seam regressions also compare complete output with exhaustive scans; ten
geometry fixtures cover terrain, interiors, lifts, materials and switched sight.
This improves construction cost without changing geometry or repairing missing
asset definitions. Traversal and rendered parity remain separate requirements.

Wychford's current descriptor (`wychford-control-audit-SHbyM9`) produces exactly
the same 28,749 boundaries with either strategy. Indexed construction takes
5.17 seconds; native loading and apply/reset of both controls takes 6.86 seconds
instead of the earlier 176.82 seconds. These are debug diagnostic timings.
All 61 ordinary native map-compilation tests pass, including rotated walkways,
terrain ramps, roof jumps, door links and lift registration.
