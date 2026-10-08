# Map compilation evidence and history

This document retains detailed checks, local artifacts and historical measurements.
Use the [map-data checklist](map-data-checklist.md) for the concise current status.
Older counts and limitations below may be superseded by later entries.

## Vertical-jump export investigation (2026-10-08)

Automatic matching of independently placed edges with `long: false` and a
100-unit elevation difference emits no pair. The export warning explicitly says
that climbing jumps require an authored connection: the clearance implementation
only models long flights. This is an authoring/export limitation, not evidence
that vertical jumps work automatically.

`node refinement/check-vertical-jump-placements.mjs` exports two separate assets
with an authored matching connection, at base elevations 0 and 40. Both exports
contain one pair and preserve the non-forced-long flag. The generated documents
and descriptors are saved together for native investigation. The first batch is
`work/map-compile/prepared-vertical-jumps-Bricsi`.

Running the existing shoulder long-jump audit against that batch does **not**
certify traversal: the ground-level fixture has no launch receiver at the tested
points; the elevated upward cases reach the long-jump-only animation assertion;
the shoulder descent cases do not complete in that harness. A dedicated audit
must distinguish upward climbing, downward dismount and long-flight animation
requirements, and use valid receivers in both placements. No runtime change is
justified by these preliminary fixture/harness results alone.

## Assisted roof jumps and carrier recovery (2026-10-08)

The prepared roof batch `work/map-compile/prepared-roof-placements-oshhrm` now
passes **120 isolated shoulder jumps and 120 shoulder-jump player-click routes**.
These use the complete RobinTown and LittleJohn animation profiles. Both actors
start linked at the source receiver. The click path selects the exported jump
overlay and uses normal group movement, which routes the carrier to the source
edge and retains the rider as owner of the jump and continuation.

Both audits originally landed the rider but left the empty helper permanently
in the carrying posture. The carrier recovery animation had no completion
state handler. On its TERMINATED edge, the runtime now restores upright/waiting,
registers the helper's wait, and clears HelpToClimb through the action-selection
handoff. A newer selected action is preserved. This is a missing animation event;
there is no added geometry computation, path preparation or navigation solver.
The direct sequence and player-click audits require the shoulder-launch animation,
release of both carry links, rider arrival and a valid receiver, and completed
helper recovery on the source sector/layer/receiver. The isolated landings are
exact; the click routes finish within 0.0001 unit of the requested point.

The full engine suite passes **4,347 tests, 0 failed, 39 ignored**. An always-on
native regression runs the complete recovery sequence with a finite animation:
DONE retains the carrying posture/action; TERMINATED restores upright/waiting
and installs Wait. Four selected/unselected action cases verify that only a newer
selected action survives the help-action deselection.

Reproduce with the existing `exported_jumps` ignored-test filter and set
`ROBIN_CARRIER_RHS` to the installed complete `LittleJohn.rhs`, in addition to
`ROBIN_CLIMB_RHS` and `ROBIN_ASSET_MAP_DIAGNOSTICS` described below. Reports are
`actor-jump-shoulders-report.json` and `actor-jump-shoulders-click-report.json`.
The four earlier upright/sword/click audits still pass (480 cases). This adds
coverage for assisted long roof jumps; helper-required vertical jumps, multi-PC
formations and rendered contact remain open.

## Shared native footprint for jump ledges (2026-10-08)

Automatic ledge generation reserved a 6×4 half-size actor while the prepared
navigation graph stores 6×3. This unnecessarily omitted an eight-unit-deep flat
roof that can contain the native footprint and a one-unit rounding margin.
The ledge generator and graph writer now share `NAVIGATION_HALF_DIAGONAL`.
Graph corner and sweep construction use the same constant; their output is
unchanged. Ledge normal/tangential clearance now uses 6×3 and retains the extra
rounding margin. A focused regression accepts the eight-unit roof and rejects
a six-unit roof with insufficient margin. Hole boundaries remain excluded.

Six checked-in native jump fixtures were regenerated from editor documents and
asset metadata. Only jump edges, zones and associated warnings changed; collision,
receivers, materials and navigation graph data are unchanged. The new ledges can
extend usable spans while the existing walking/flight obstacle checks still trim
them. The focused editor/compiler suite passes **246 tests**; native map integration
passes **71 tests, 5 ignored**. The refreshed fixtures also pass **85 native jump
tests, 5 ignored**. App typechecking and scoped type-aware lint also pass.

Fresh batch `work/map-compile/prepared-roof-placements-oshhrm` passes **120 isolated
upright jumps, 120 complete upright approaches, 120 isolated sword jumps and 120
player-click routes**. Isolated jumps and interior clicks arrive exactly. All
four-unit approach cases finish the finite turning startup 0.8 or 1.6 units short,
verified by the step-distance and command-completion audit described below.
The two 90-degree cases still omit connections with explicit receiving-span
warnings; this change does not certify those omissions or full placement coverage.
No production runtime code changed. Assisted jumps, complete combat interactions,
multi-PC click formations and rendered contact remain open.

## Prepared roof jumps and continuous receivers (2026-10-08)

`node refinement/check-roof-jump-placements.mjs` (from `level-editor/`) exports
two independent copies of a complete two-house assembly at 0/37/90/180 degrees
and heights 0/40. It reads the editor/asset fixture only and records omitted
connections and warnings in its manifest. It does not certify gameplay itself.
Six placements retain two pairs each. At 90 degrees the projected sloping roof
has no generated character-sized receiving span, so both height cases omit the
connections with warnings; those omissions remain outside traversal coverage.

The native sprite audit initially found four landing-receiver mismatches in
120 directed edge-position cases. A pair of consecutive vertices collapsed to
one binary32 vertex, unnecessarily triangulating the otherwise simple receiving
floor. Jump landing uses the receiver at the edge midpoint, so that split left
an actor with a different receiver at an endpoint. Export now removes consecutive
vertices only when all four stored coordinates become identical in binary32.
It retains genuine pinches and their triangulation, exact native vertices,
receiving planes and materials. No production engine code changes.

Fresh batch `work/map-compile/prepared-roof-placements-FzNuR1` passes all **120
isolated sprite jumps/landings**. The checked-in rotated-roof descriptor matches
editor output and passes **84 native receiver queries** along its four directed
edges. Native actor-sized approach/pathfinder checks include that descriptor.
Validation: **101 focused editor/compiler tests**, **71 native map integration
tests (5 ignored)** and scoped type-aware lint pass.

The animation audit now accepts an explicit per-map `approach_depth`; its default
remains eight units. This fixture specifies its authored four-unit landing depth.
It first checks both swept approaches using the native actor footprint. All 120
cases pass that geometric check. Initially **112/120 complete walk–jump–walk
sequences failed the 0.1-unit final-position assertion**, stopping 0.8 or 1.6 units
short after the jump. Tracing established that these moves consist solely of the
finite walking-start animation: turning reduces each step to 60% of its distance,
and the command ends when that animation finishes if no subsequent movement order
exists. This matches the reference behavior. A controlled native test checks four
units of requested travel with no turn, one turning step and two turning steps:
the resulting distances are 4.0, 3.2 and 2.4, respectively.

The audit now distinguishes exact arrival from that completed turning startup.
It verifies every observed startup displacement against its frame distance,
checks that the unscaled animation could reach the goal, rejects blocking and
deviation, requires animation and command completion, and checks the calculated
final point within 0.001 unit. This is not a blanket near-goal tolerance. Reports
retain the actual final offset and measured turning loss. **All 120 complete
walk–jump–walk cases now pass**: 8 exact arrivals and 112 native turning completions.
The 120 isolated jumps still land exactly. No production engine behavior changed.
The full engine suite passes **4,346 tests, 0 failed, 37 ignored**.
The earlier eight-unit audit additionally
requested points outside this fixture's guaranteed landing band and is not proof
that the authored four-unit approach is obstructed.

The same batch also passes **120 isolated sword-jump dispatch/landing cases**.
Each case installs an opponent relationship, enters the native combat state and
executes the complete sprite orders. It requires an observed sword-flight
animation, exact landing, the correct receiver/layer/sector, a return to the
waiting-with-sword state and preserved opponent relationships in both directions.
This checks prepared connections with the dedicated combat animation; it does
not certify combat AI, player-click authorization or an entire combat approach.
Player clicks use upright walking/running for their approach and continuation,
so substituting sword walking is not an equivalent click test.

The player-click audit passes **120 complete routes**, all with exact final
arrival. It uses `get_sector_screen` and normal group-movement dispatch without
supplying a jump-line, gate-route or destination-sector override. Before dispatch
it verifies that the hit is a jump overlay and that native skill/posture
authorization selects the expected connection, including its independent copy.
It then ticks sequence selection, pathfinding and complete character animations.
Clicks sample five interior positions (5/25/50/75/95% along each edge) at half the
authored landing depth. Full-depth points lie on the outer band boundary and can
fall outside after integer rounding; those remain covered as explicit movement
goals by the separate approach audit, not as guaranteed jump-overlay clicks.
The checked-in rotated-roof fixture also tests midpoint overlay selection in
both directions for both copies, and rejects each route after removing jump skill.
That check requires no external sprite installation.

Reproduce the four external sprite audits using `cargo test -p robin_engine --lib
exported_jumps -j 1 -- --ignored --nocapture`, with `RUST_MIN_STACK=33554432`,
`ROBIN_ASSET_MAP_DIAGNOSTICS` set to the generated directory, and
`ROBIN_CLIMB_RHS` pointing to the installed complete `RobinTown.rhs`. These checks
do not certify assisted dispatch, complete combat interactions, multi-PC click
formations or rendered contact. Reports include `actor-jump-sword-report.json`
and `actor-jump-click-report.json`. No production runtime code changed.

## Spline clearance navigation planes (2026-10-08)

`refinement/check-spline-clearance-planes.mjs` builds eight maps entirely from a
synthetic wall asset and authored terrain: flat/30-unit-rising paths, base heights
0/40, and clearances disabled/enabled. Each wall repeats three times; each raised
cutout retains a central solid pillar. Physical cutouts sit thirty units above
their independently retained navigation plane. Twelve line probes per map check
the two open sides, central pillar and adjacent wall.

The first native batch exposed missing collision even with clearances disabled:
independently rounded spline XY/Z coordinates left a sloped underside slightly
above its matching terrain. Contact matching now recognizes a plane coincident
over the full footprint within 1/1024 map unit; clearance matching compares actual
height differences over its contour rather than raw coefficients. A unit check
retains a genuine 0.01-unit underpass and excludes a supporting slab below a floor.

Batch `spline-clearance-planes-i1SuhK` passes all eight native constructions and
96 line probes; 173 focused compiler/spline/physical-region tests pass. This is
native thin-path geometry coverage, not complete actor walking, curved-path
native traversal, or rendered contact. No engine runtime change was required.

Follow-up `spline-clearance-planes-PgrCSo` widens each side of the cutout while
retaining the central pillar. All eight constructions and 96 thin probes pass,
plus 48 directed native pathfinder routes through the openings. Route endpoints
must fit the normal movement footprint, and each forty-unit crossing has a
45-unit maximum path length so walking around the wall cannot satisfy the check.
Both directions pass on flat/rising terrain at elevations 0/40. This exercises
actor-sized routing, not animation ticks or full actor motion execution. The
external native regression manifest now accepts optional `route_probes[].max_length`.

## Independent activated solid fragments (2026-10-08)

Removable Croisement03 fragments 098/099 are authored by
`refinement/stage-croisement03-removable-fragments.mjs`, pinned in
`refinement/catalogs/croisement03-removable-fragments-gameplay.json`. Each starts
with its local footprint and sight volume enabled, then removes them with its
descriptor-owned hide binding. The assembly's larger applied contour extends
beyond these parts and is not inherited. Stage
`croisement03-removable-fragments-e49PrS` passes eight native constructions,
32 apply/reset checks and 96 thin-route probes across copied placements at four
rotations and two elevations. Routes are blocked initially, open after removal,
and blocked again after reset. This does not simulate full actor walking.

Publication backup `croisement03-removable-publication-e49PrS` contains both
descriptor changes; no model or saved-scene changes are needed. Published browser
archives `croisement03-fragment-098.zip` and `croisement03-fragment-099.zip` pass
overlapping-copy insertion, independent visibility, save/reopen, color/depth
bake and native ZIP construction without base data. Both carry two controls in
one region with four states. Audit `gameplay-coverage-OhuGjZ` reports 21 missing
definitions, 17 with known sight-state dependencies, and zero missing placed
definitions in the ten saved scenes. Full mesh-contact and actor review remain open.
Both archives also pass OpenGL GPU color/depth probes for each control through
all four combinations, transition suppression and reset. Patch flags are assigned
directly, so this checks rendering independently of actor-triggered activation.

Croisement03 fragments 103/104/105 now have independent drafts authored by
`refinement/stage-croisement03-activated-fragments.mjs` and pinned in
`refinement/catalogs/croisement03-activated-fragments-gameplay.json`. Each has
its own footprint exclusion, applied sight volume and descriptor `show` binding.
No shared assembly exclusion or remote trigger is inherited. Fragment 102 is
excluded because it also owns persistent collision in the reviewed assembly.
Stage `croisement03-activated-fragments-kZ4YpX` passes eight native constructions,
48 apply/reset checks and 144 thin-route probes at rotations 0/37/90/180 and
elevations 0/40 with independent copies. Each map has six controls, seven sight
volumes and 1,344 grid blocks. These are navigation probes, not complete actor walks.

Publication backup `croisement03-fragments-publication-kZ4YpX` changes only the
three descriptors and regenerated index; no saved scene pins or meshes change.
Browser fixture archives `croisement03-fragment-103.zip`, `104.zip` and `105.zip`
(each with the full `croisement03-fragment-` prefix) pass copied/rotated overlap
preview, save/reopen, color/depth bake, ZIP packaging, native construction without
a base datadir, and OpenGL appearance/depth probes. Each has two controls in one
region with four visual states. GPU flags are assigned directly; actor activation,
physical mesh contact and full gameplay parity remain unverified. Current audit
`gameplay-coverage-Qd1DCH` finds 23 missing definitions, 19 with known state-owned
parts, and zero missing definitions referenced by the ten saved scenes.

`refinement/stage-croisement-activated-fragments.mjs` authors standalone gameplay
for `croisement01-group-083` and `croisement01-group-084`. The reviewed assembly
has no initial sight or movement geometry and activates both parts together.
Each new definition instead owns a single control with only its own sight volume
and a ground exclusion derived from that volume's local footprint. Its waypoint
is the local footprint center. The shared assembly contour and remote waypoint
are not copied. Compilation reads only the resulting descriptors and editor
document; no level records or assembly lookup are needed at runtime.

Stage `activated-fragments-mtvhIM` places two copies of each fragment at fractional
translations, rotations 0/37/90/180 and terrain elevations 0/40. All eight maps
construct natively with four independent controls, five sight obstacles including
the terrain receiver and 960 grid blocks. All 32 control apply/reset checks pass.
For each control, a thin route crossing its footprint is reachable initially,
blocked when applied and reachable after reset: 96 native route probes in total.
The audit also checks state restoration and the other controls' unchanged sight
and navigation state. These are native grid probes, not complete actor walks.

Published catalog `refinement/catalogs/croisement01-activated-fragments-gameplay.json`
pins both descriptors and models. Backup is `activated-fragments-publication-mtvhIM`;
no saved scene pins or models change. Fresh published batch
`activated-fragments-published-OEiZ2O` reproduces all eight tested descriptors
exactly, writes editor documents with current pins, reopens them through the
library loader and recompiles identically. All ten saved scenes also reopen.
The regenerated index audit `gameplay-coverage-iQKmEx` reports 26 missing
definitions, 22 with known state-owned parts, and zero missing placed definitions.

Both definitions remain explicit drafts. Their initial state is unapplied;
activation must be authored explicitly. Physical mesh contact, complete actor-route
validation and a rendered in-game playthrough are still open. This publication
does not claim exact reproduction of the shared assembly's hand-authored movement
contour or full gameplay parity. No engine changes were needed.

The separate `croisement01-activated-fragments-appearance.json` catalog adds
descriptor-owned `show` rules bound to each fragment's local activation control.
The pinned dry-run backup is `activated-fragment-appearance-pinned-review`;
descriptors are installed without mesh edits or saved-scene changes. Browser
coverage uses `asset-endpoint-insertion.html?fragments=1`: actual library models,
two copies per asset, one rotated 37 degrees, save/reopen, independent apply/reset,
changed color and depth images, and packaged appearance resources. A real-loader
failure exposed the index dropping `appearance`; index projection now retains it
in base and variant parts, covered by the 36 index/publication tests.

`activated-fragments-browser.zip` contains the 084 case. The existing native
`full_editor_archive_constructs_native_map_without_base_datadir` check loads its
2000×2000 color/depth/minimap and editable scene, then constructs three sight
obstacles, 3,456 grid blocks and two appearance regions for two controls without
base game data. This does not assert final renderer composition or actor behavior.
`activated-fragment-083-browser.zip` passes the same native check and counts;
its browser run also verifies all four independent preview control combinations.
The three existing drawbridge browser insertion cases also still pass.

The native GPU gate now accepts `ASSET_APPEARANCE_EXPORT_ZIP` directly. Both of
these browser archives pass with `WGPU_BACKEND=gl`: two regions/two independent
controls, all four state combinations, transition suppression and reset. At a
pixel in each region where both baked color and depth change, actual GPU color
matches decoded RGB565 and a sprite probe is correctly hidden/shown on opposite
sides of that state's occlusion depth. The gate mounts only the ZIP and assigns
patch flags directly; it does not test actor-triggered callbacks or complete
scene compositing. Vulkan cannot run here: adapter discovery reports no suitable
adapter. No outside-sandbox request was made. The Rust library test build and
both OpenGL runs pass; only test code changed.

The `&overlap=1` browser variant places the second, rotated copy 25 units from
the first. Both `activated-fragment-083-overlap.zip` and
`activated-fragment-084-overlap.zip` contain one shared appearance region with
four states for two controls. Browser insertion/save/reopen/preview/bake/package,
native archive construction and OpenGL pixel checks pass. The GPU gate now probes
a color/depth-changing pixel for every individual control, rather than one per
region, and checks those pixels through all four combinations and reset. This
tests overlapping authored assets without reproducing a saved game map.

## Missing-definition ownership dependencies (2026-10-08)

Fresh inventory `gameplay-coverage-0U9BM2` validates all 1,279 indexed descriptor
hashes and finds 28 missing gameplay definitions, none referenced by any of the
ten saved scenes including spline sources and corner assets. Twenty-six have
matching part-name owners, but 24 also contain parts listed in those owners'
initial/applied sight-state sets. The inventory now reports each candidate
owner's transition IDs and the initial/applied membership separately. Its unit
check covers initial-only, applied-only, shared and unrelated parts.

For example, the standalone York canvas screen contains `building-965` from the
market assembly's initial state and `building-977` from its applied state.
Assigning static collision to both would lose state selection. Native patch
application toggles the two sets separately, consistent with the established
behavior. This is evidence for dependency-aware authoring, not authorization to
copy an entire assembly or a claim that matching node names share local frames.
No gameplay definitions or assets were published by this audit.

The four entries without recorded sight-state membership are two background-art
assets, `derby-great-keep` and `nottingham-castle-watchtower`. They still need
review of their geometry, local frames and non-sight dependencies; an empty
membership list does not certify an independently usable asset.

## Closed spline mask contours (2026-10-08)

Follow-up regression catches vertices that boolean clipping removes as collinear
in its 2D projection even though they carry bends in the third coordinate. The
compiler restores surviving source vertices before inserting spline stations.
Forward and reversed contours retain both the four-unit peak and the two-unit
interpolated midpoint. All 33 focused compiler/editor checks pass, including
native-fixture equality and editor reopening; typecheck, changed-file lint and
the production build pass.

Fresh Wychford descriptor `saved-map-exports-ROPd6Z` is exactly equal to the
native-tested published descriptor `saved-map-exports-WcPgWR`, including warning
text. Export takes 137.72 seconds and retains 1,869 warnings. The Chromium
`map-bake.html` contract passes crop alignment, tile seams, hidden geometry,
color/depth encoding, state apply/reset, automatic appearance regions, bounded
sun shadows, resource restoration and ZIP/PNG round-tripping. Its fresh archive
is `work/map-compile/closed-mask-bake-contract.zip`; this is the generic synthetic
bake fixture, not a rendered closed-mask-island scene or full Wychford bake.
The native browser contract now accepts a fresh archive through
`ROBIN_EDITOR_BAKE_CONTRACT_ZIP`, preserving the checked-in archive as a separate
regression. Both archives pass discovery, mounting, color/depth/minimap decoding
and live engine navigation/sight construction without a base datadir. The fresh
archive uses `spawn_points: []`; the older fixture retains `spawn_player: false`.
Only that descriptor field differs between their unpacked contents. No runtime
behavior was changed for this acceptance check.

Closed concave application contours previously used single-ring strip clipping,
which could connect surviving islands along the trimmed edge. The compiler now
clips in a spatial projection that includes the crop axis, retains separate
closed components, restores the third coordinate from source contour edges and
adds the spline's bend stations before deformation. Uncropped boundaries retain
their original points. Each island gets an independent character/projectile
application; coverage is shared and view/obstacle rules remain single per repeat.
No engine behavior changes are required.

The synthetic closed-neck fixture produces six masks across three repeats.
Its compiled masks exactly match `asset-spline-closed-masks.json`, and exporting
the embedded editor document, reopening it and recompiling preserves the entire
descriptor. Independent-island comparisons pass at 0, 37 and 180 degrees on
bent, rising paths. Helper tests cover both crop axes, reversed winding, sloped
and vertical contours, complete removal and a boundary-only touch.
All 32 focused editor/compiler tests pass. The native fixture checks both mask
application rules for 21,312 mask/point combinations, including island and repeat
edges, and verifies shared bitmap coverage plus independent obstacle ownership.
All 68 enabled native map-compilation tests pass; five external-data audits are
ignored by that suite.

App typecheck, production build, changed-file lint and formatting pass. Full
lint reports five findings in unchanged state-preview/catalog files; it does
not pass. Rendered character occlusion and full browser ZIP baking remain
separate unfinished checks.

## Exported jump diagnostics (2026-10-08)

Contact review recipe `refinement/stage-watermill-jump-contact.mjs` varies the
published jump's standing inset and ground separation without changing its
floor, solids, jump zones, span length or terrain heights. Fifteen candidates
cover insets of 1, 2, 3, 4 and 6 units and extra ground offsets of 0, 20 and 40
units. Each is compiled at four rotations and two elevations with two copies,
then checked for 480 flight paths, 160 walk–jump–walk routes and 64 zone endpoints.
The eight disconnected-terrain negative cases also run for every candidate.
Results are in `work/map-compile/watermill-contact-review-results.json`.

No candidate passes all three positive checks. At zero extra ground offset,
the five insets respectively produce 0/12/24/76/80 flight failures and
88/76/68/16/0 walking failures. A 20-unit ground offset clears flights for the
2- and 3-unit insets but leaves their walking failures. A 40-unit offset clears
flights at the 4-unit inset but still fails 16 walking routes and puts 32
endpoints outside their jump zones. Keeping the published 6-unit inset retains
all walking routes, but the 40-unit offset still fails 20 flights and the same
32 zone endpoints. These candidates are unpublished; none replaces the current
asset. This bounds the tested adjustment space, not every possible asset design.

The external native flight audit now records every case in
`actor-jump-flight-report.json` before failing for intersections. Each result
identifies file, pair, direction, edge position and movement style, with the first
blocking segment for each obstacle in both planned and frame-integrated paths.
It retains strict failure: completing the audit does not mean clearance passed.
This change is confined to tests; gameplay behavior is unchanged.

The published watermill batch `watermill-platform-placements-0kbdpB` reports
80 failures out of 480 cases. All failures are uphill sword-fighting flights,
and all intersect in both planned and frame-integrated paths. All 160 upright
and 160 assisted trajectories plus 80 downhill sword flights clear active solids.
Assisted trajectories are not complete assisted-sprite dispatch. The earlier
boundary-only batch `watermill-platform-placements-6bZOzc` passes all 480 cases
as a positive clearance control, but retains its previously measured walking
failures. Nine checked-in geometric-jump fixtures also pass the expanded audit.
The straight sword flight is the intended movement model, so this evidence does
not justify changing engine flight behavior to conceal the asset intersection.

The watermill is now published as a best-effort draft using
`refinement/catalogs/leicester-watermill-walking-gameplay.json`; backup and
reviewed before/after files are in `watermill-publication-MPjCH9`. The descriptor
is `1698ac81ef7d101986e6809e0902538a4b02d15e19f4f57bcf98c9f60811138c`;
the reviewed model remains unchanged. Leicester and Wychford scene pins are
updated, the library index is rebuilt, and all ten saved scenes reopen.
Published placement batch `watermill-platform-placements-0kbdpB` matches the
eight native-tested `watermill-platform-placements-jF0VUC` descriptors exactly
apart from draft warning text. Unsupported low-terrain cases still reject.
The published batch also passes all 48 complete-sprite entrance round trips.

Full saved-scene candidate batch `saved-map-exports-H9pASL` passes native
construction (277.63 seconds for both maps) and all fifteen control apply/reset
checks (286.13 seconds). Wychford has 154 motion areas, 16,185 sight obstacles,
31 doors, no jump pairs, 62,244 grid blocks and 36,790 elevation boundaries.
Leicester has 57 areas, 826 sight obstacles, 105 doors, 23 jump pairs, 65,170 grid
blocks and 715 elevation boundaries. Wychford retains the mill's raised floor
and upper entrance; two lower entrances and its jump remain omitted because
the placed asset lacks ground contact. These are descriptor-level checks,
not a browser ZIP bake, rendered review or full playthrough.
Fresh exports from the published library (`saved-map-exports-WcPgWR`) match both
native-tested descriptors exactly after excluding only warning text. Wychford
exports in 141.42 seconds with 1,869 warnings; Leicester in 2.12 seconds with
401 warnings. Updated draft wording accounts for the warning-count change.

Authored pairs now use the same upward integer height rounding as generated
ledges (including their 0.0001 near-integer tolerance and positive zero).
This prevents ordinary nearest rounding from putting a flight endpoint below
its fractional supporting surface. Projected contact coordinates do not move,
and integer-authored heights remain unchanged. Fractional positive/negative
height tests and all 160 focused compiler/clearance tests pass.

Review recipe `refinement/stage-watermill-walking-jump.mjs` derives an authored
pair from the boundary-ledged candidate. It adds a five-unit standing inset,
shortens the span at both corners and gives the wider corner five further units
of clearance for inward continuation. It preserves the floor and solid volume.
Stage `watermill-walking-jump-MPjCH9`, batch `watermill-platform-placements-jF0VUC`,
passes all 160 upright walk–jump–walk routes and all 160 isolated sprite jumps.
All 64 endpoints lie in their home zones; eight low-terrain negative cases still
omit unsupported lower entrances and jumps. Compilation reads only the staged
asset definitions and assembled editor document.

The full airborne audit still fails the straight combat return path against
solid platform obstacle 0, starting at map point (812, 889). Compiler warnings
retain this limitation. This is a published best-effort draft,
not proof of combat-animation or rendered-contact parity. The earlier boundary
candidate's 480 clear airborne paths must not be attributed to this revision.

Parallel authored jump spans now round as a pair. The second edge's A endpoint
is derived from its rounded B endpoint and the first edge's rounded vector;
the separate height-rounding rule above still applies. Only spans already parallel and opposite before
quantization take this path. Nonparallel authored pairs still report their
alignment problem. Fractional placement checks at 0, 37, 83 and 143 degrees
verify vector equality and unchanged endpoint heights.

Three diagnostic-only inset probes of the watermill export isolate the remaining
walking/flight tradeoff. Moving the upper line inward by another 3, 5 or 7 map
units produces 88, 116 and 116 walk–jump–walk passes respectively. These are
descriptor experiments, not asset-derived publication candidates. The five-unit
probe (`watermill-inset-5-probe-cdzj_iex`) fails native solid clearance on the
straight sword-fight return path. Increasing the inset alone is not a complete
fix, and neither geometry nor collision checks have been relaxed to hide it.

The failed walk-away recovery now has a standalone geometric regression. A
fractional diagonal platform rejects the direct recovery sweep because it adds
a triangular overhang outside the actor's initial footprint. The same supported
endpoint is reachable by an axis-aligned two-leg path without introducing any
new unsupported area. Recovery now tries those two inward paths when the direct
sweep fails. Each leg must keep its center on the floor, avoid solids, and keep
the swept footprint inside existing support plus the initial overlap. The
endpoint displacement limit and receiving height remain unchanged.

This runtime correction raises `watermill-platform-placements-6bZOzc` from
12/160 to 56/160 walk–jump–walk passes. All 160 isolated upright sprite jumps
still pass. The standalone regression checks the displacement limit and support
after recovery; all 45 floor/navigation and 55 enabled compiled-navigation tests
pass (20 external audits remain ignored in that suite). Remaining route failures
and publication requirements are still open.

Follow-up authoring recipe `refinement/stage-watermill-boundary-jump.mjs` replaces
the inset watermill pair with a geometric ledge on the platform's outward end
and an asset-local ground counterpart. The ground ledge is wider so oblique
placements retain a shared span after projection; matching remains bounded by
100-unit gap and 20-unit rise/drop limits, with a two-unit minimum shared span.
The compiler clips the match and tests clearance after grid conversion. The upper
jump zone now uses the actual platform footprint, avoiding the old zone's roughly
one-unit inset. This is a review candidate, not a published asset change.

Stage `watermill-boundary-jump-p7GHo6` produces placement batch
`watermill-platform-placements-k4wLaB`. All eight maps retain two jump pairs and
six entrances; lower-terrain negative cases still omit both pairs and the four
unsupported lower entrances. All 64 line endpoints lie in their home jump zones.
The native flight audit passes 480 paths, covering both directions, five edge
positions, upright/sword/assisted flight, two copies, four rotations and two
elevations. Planned paths and frame-integrated paths clear active solid geometry.
This does not exercise full sprite dispatch, ordinary approach or landing
receiver publication; those and rendered contact remain required.

The new ignored native audit
`exported_jumps_complete_sprite_dispatch_and_land_on_receivers` uses the complete
character sprite at five positions on every directed line, including both ends.
It requires receiving floors at launch and landing, observes airborne posture,
waits for the complete order chain, and checks destination sector/layer, receiver
identity and physical height. It starts on the launch line and does not exercise
the click/approach route. Reports are written as `actor-jump-landing-report.json`.
The boundary-only batch above fails 104 of 160 cases in this stronger audit.

The staging recipe now insets the ledge one unit into the floor and trims two
units from each corner before geometric matching. Stage
`watermill-boundary-jump-hAHGgJ`, batch `watermill-platform-placements-6bZOzc`,
passes all 160 upright sprite-dispatch/landing cases and all 480 airborne-path
checks. All 64 endpoints remain in their home zones. These asset-only changes
preserve the receiving plane and native jump behavior. Walking approaches,
sword/assisted complete sprite dispatch and rendered contact remain unverified.

The additional ignored audit `exported_jumps_walk_to_launch_and_continue_after_landing`
constructs the normal line-goal approach, jump command and following walking
command, using the complete upright sprite. It starts eight map units inside
each launch edge and requests eight units of movement past the paired edge.
Five edge positions in both directions across the watermill batch above yield
12/160 passing routes. Eight source probes and eight destination probes lack
receivers; 132 routes terminate before reaching the final goal. The actor's
selected wait command and physical support/recovery results are included in
`actor-jump-approach-report.json`. A representative approach source is supported,
but a representative return landing has neither full footprint support nor a
local supported recovery. No runtime behavior has been changed for this audit.

Control batch `jump-approach-control-8rxss1r_` runs the same sequence on existing
fixtures. The complete-roof fixture passes 9/10 routes; its remaining endpoint
does not finish the final walk. The smaller geometric fixture has no receiver at
the chosen eight-unit approach probes and is not a positive walking control.
These results establish a walking-clearance gap beyond successful isolated
sprite jumps; they do not certify all approach probes or click authorization.

All compiled jump pairs now receive a diagnostic pass after integer conversion,
including authored pairs and exact socket joins. Opposing map-space vectors must
match for distance-along-source translation to cover the paired landing edge.
Mismatches produce asset-identified warnings. Aligned pairs also run the existing
flight/takeoff clearance calculation against solid geometry, including geometry
that can become active after a state change. Unsupported sloped/climbing checks
are explicitly reported as unverified. These diagnostics preserve the exported
connections and do not substitute warnings for a future geometry correction.

The watermill batch `watermill-platform-placements-G273sd` retains identical
geometry in all eight maps compared with `watermill-platform-placements-A7VFwM`.
Each now reports the two copied authored jump-vector mismatches. All placement
and lower-terrain assertions pass. The focused compiler and clearance suites
pass 158 tests, including fractional-solid takeoff collision, unequal vectors,
and unsupported flight geometry; editor typechecking passes.

## Native receiving-floor loading cost (2026-10-08)

The completed Wychford export `saved-map-exports-e13P3h` passes native construction
and all three control apply/reset checks. Construction retains 153 areas,
16,185 sight obstacles, 30 runtime doors, no jump pairs, 59,850 grid blocks and
36,782 elevation boundaries. The two debug audits took 774.52 and 773.68 seconds.
These successful but slow checks do not resolve the export's gameplay omissions.

Reduced copies in `native-receiving-profile-rzuwurkb` retain the first motion area
and prefixes of its actual receivers. All four construct successfully. Descriptor
decoding takes 11–283 ms; engine construction takes 1.51, 5.90, 23.94 and 48.10
seconds for 32, 128, 512 and 1,024 receivers. Optional
`ROBIN_ASSET_MAP_PROFILE=1` timing isolates ordinary floor binding: 32 floors
take about 1.31 seconds, while grouping takes 0.37 ms and neighbours 3.23 ms.
One validation of the 530-vertex motion boundary takes about 37 ms; that work
repeats after unprojecting the boundary for each plane.

Large single-ring physical polygons now sort edge bounds and skip spatially
disjoint pairs before the same exact intersection and shared-endpoint checks.
Geometry, tolerances, plane coefficients and routes are unchanged. Rings with
holes and small rings retain the existing validator. Dense f32/f64 valid,
crossed, repeated, reversed, degenerate and nonfinite cases match its results.

The four reduced construction times fall to approximately 0.112, 0.437, 1.768
and 5.107 seconds; the complete reduced audit falls from 79.96 to 8.12 seconds.
All 44 stair-navigation, 55 enabled compiled-navigation and 67 enabled native
map-compilation tests pass. Eighteen compiled-navigation and five map-compilation
audits require external fixtures and remain ignored in those suites. Three
movement tests now clear obsolete plane anchors when replacing fixture vertices,
and clear obsolete precise boundaries when replacing the motion area; their
behavioral assertions are unchanged. The same full Wychford export passes
construction in 255.00 seconds and all three control apply/reset checks in
260.72 seconds after optimization. These are single-run debug audit timings,
not release-game loading measurements.

## Watermill physical platform candidate (2026-10-08)

The remaining Wychford receiver omission is asset-local: the watermill has a
raised receiving volume but no walkable surface. Its foundation navigation
anchor is about 90.7 units above the containing terrain in that saved placement.
Automatically binding it to that terrain would conceal a real height gap.

Mesh review `watermill-platform-review-rjec6W` binds descriptor
`b008913a237417aadef280b0ac3ab8835ea69482aeefa81fc0055833c153c01b` and model
`ba0b9fc1d5047e0bffc876e357408270651414e3d31c8190bee86466a133dc11`.
Only four triangles lie within 0.01 units of the authored top, but eight lie
within 0.1 units and cover nearly the complete platform. The authored footprint
is 5,886.206 square units; 3.431 square units remain outside that sampled mesh.
Raising the tolerance to five units does not materially reduce that remainder.

`refinement/stage-watermill-platform-floor.mjs` stages an unpublished replacement
that gives the physical platform its own walkable receiver and preserves both
existing material regions. Its boundary connections require matching placed
heights. Stage `watermill-platform-floor-pOWroc` compiles Leicester into
`saved-map-exports-GUqGef`, which passes native construction: 57 areas, 826 sight
obstacles, 105 runtime doors and 23 jump pairs. All eight platform boundary
joins remain independent in this scene; no watermill door or jump omission is
reported. This is construction evidence only.

The placement verifier `refinement/check-watermill-platform-placements.mjs`
places two copies at rotations 0, 37, 90 and 180 degrees and elevations 0 and
40. The initial candidate passes 32 of 48 complete-sprite entrance/exit routes;
all 16 upper-door routes fail because the handoff point is outside the platform.
The staging recipe now moves that asset-local point along its existing approach
onto the platform, with a 1.5-unit inset for integer waypoint rounding. It changes
no door heights or runtime behavior. Stage `watermill-platform-floor-LA5GzH`
matches the tested candidate in `watermill-platform-placements-3EojSS`, where
all 48 routes pass. Eight negative cases with terrain 20 units below the asset
retain only the two supported upper entrances and omit the lower entrances and
jump pairs (`watermill-platform-placements-scLf10`).
All eight placement descriptors match the actor-tested batch exactly. The corrected
full Leicester export `saved-map-exports-NFGMKR` also passes native construction
with the same geometry counts as the initial candidate, in 0.56 seconds.

The expanded native airborne-path audit fails on the first positive placement:
a jump from map point (772, 921), height 12, intersects solid platform obstacle
0, whose top is 12.001. The collision and quantized endpoint handling remain
under investigation. An isolated diagnostic raising that upper endpoint to 13
gets past the first collision but fails the landing correspondence assertion:
the stored opposing edges have unequal integer vectors, whereas runtime landing
translation uses distance along the source edge. This diagnostic is not an asset
fix and has not been published. Authored pairs currently bypass the clearance
checks used when assembling connection segments; that compiler gap needs review.
Actor doorway success does not certify jump execution,
rendered contact or complete gameplay parity. The candidate remains unpublished.

## Dense receiving-boundary construction (2026-10-08)

Captured Wychford inputs in `receiving-profile-DCBnzh` isolate the slow operation:
the first navigation region joins 850 floor pieces containing 4,449 vertices.
Transition construction takes about 9 ms; the following fixed-point outer union
is expensive. Prefixes of 64, 128 and 256 pieces take approximately 42, 736 and
9,292 ms. Batching 32 pieces reduces those unions to 21, 87 and 157 ms, and
the full 850-piece boundary operation completes in about 3.1 seconds.

This is not bit-identical polygon arithmetic: the 128/256-piece unions differ
from the single fixed-point union by 0.000001811 square game units. The complete
850-piece batched union differs from a double-precision reference union by
0.000806139 square units at the fixed-point comparison precision. A direct
floating-point XOR of those complete unions fails in the clipping library;
the fixed-point comparison succeeds. No complete baseline union was obtained.
A 504-triangle regression verifies the fractional perimeter and preserved gap.

The resumed export then exposed a floating-point intersection failure during
obstacle recovery. Its captured five-vertex candidate and six-vertex source
reproduce the exception. Recovery now retries that operation at the fixed-point
precision used for source collision; a regression checks both source containment
and identical rounded obstacle coverage. This does not change engine code.

Broader export checks found two stale fixtures that also fail with the unchanged
contour implementation. Refreshed platform-opening and rotated terrain-socket
fixtures pass the native receiver-hole test and all ten directed actor crossings
at five rotations. All 67 enabled native map-compilation checks pass; five
data-dependent audits remain ignored. The targeted editor suites, typecheck,
lint, formatting and production editor build pass.
The game binary also builds and Rust formatting passes. Full Wychford descriptor
export `saved-map-exports-e13P3h` completes in 126.2 seconds, including one
best-effort retry for its disconnected church-tower approach. It has 1,862
warnings, including one omitted watermill receiver, 25 door warnings and 13 mask
warnings. Compilation success does not certify those missing connections or a
browser ZIP bake.

## Published centered three-entrance ladder (2026-10-08)

`refinement/catalogs/sherwood-ladder-oak-centered-gameplay.json` records the
published draft replacement, bound to reviewed descriptor and model hashes.
It reconstructs the platform opening, supplies matching movement clearances and
a receiving volume, and centers the left upper approach along its authored
flight edge. The three approach waypoints move together by
`[-2.8565409062500464, -0.43463385174097446]` without changing their heights.
All definitions remain asset-local and follow placement transforms.

The uncentered landing-strip candidate passed only 24/48 complete-sprite routes
in `local-stair-placements-omaXjD`. The centered candidate passes all 48 in
`local-stair-placements-OTv6aK`, covering rotations 0, 37, 90 and 180 degrees at
elevations 0 and 40. Each placement retains three entrances and physical
navigation; all eight raised-ground negative cases reject the disconnected
ladder. This correction needs no engine implementation change.

Full Sherwood stage `saved-map-exports-yQPQJi` passes native construction,
ten complete-sprite climb routes and two stair routes. The state-control audit
reports zero transitions and fails its nonempty-coverage guard; it provides no
state-transition evidence for this scene. Published export
`saved-map-exports-zVBfvB` is byte-identical to the tested staged descriptor
(SHA-256 `4e60fd7ccd42c29d168ae4f9b8b7fe69098f73386595461c794c6d2056f0dbe3`).
All ten saved scenes reopen with current descriptor pins. Five publication
tests pass, including stale-model rejection and rollback.

This is best-effort traversal support, not full rendered parity. The left upper
approach exceeds sampled platform mesh by up to 2.366 game units
(`centered-ladder-landings-lfIQrN`). Only two of 28 selected rung parts cross the
authored climbing plane; maximum bounding-center distance is 12.012175 units
(`climb-mesh-audit-IR3yV4`). These measurements do not establish actor-foot
contact. Both limitations remain explicit draft warnings, and the full export
still reports 258 warnings across the scene. Publication backups are in
`work/map-compile/centered-oak-publication-review`.

## Earlier three-entrance ladder recheck (2026-10-08)

The unpublished mesh-platform candidate `three-door-oak-7osA0K` still matches
the current `sherwood-ladder-oak-platform` descriptor hash. Fresh compilation
`local-stair-placements-Vxn5ch` retains all three doors at 0, 37, 90 and 180
degrees and elevations 0/40, and all eight raised-ground negative cases reject.
Every placement reports physical navigation unavailable because the left upper
landing does not reach its ladder door; projected navigation is retained.

The freshly built native complete-sprite audit passes 36/48 directed routes.
All twelve failures occur at 90 degrees, at both elevations, and stall after
entering the lift. This reproduces the unresolved failure with the current
compiler and runtime; the candidate remains unpublished.

Pinned-model audit `climb-mesh-audit-IR3yV4` selects 28 rung parts. Only two cross
the authored infinite climbing plane; the maximum bounding-box-center distance
to that plane is 12.012175 game units. These are mesh-plane measurements, not
actor foot-contact measurements. They do not justify enlarging a navigation
surface across the platform opening. The next authoring work must reconcile the
climbing surface and all three landing contacts with the asset mesh. No engine
extension or library publication was made during this recheck.

## Batched receiving-material unions (2026-10-08)

Receiving-material partitioning previously unioned each equivalent patch into an
ever-growing region. For 128, 512 and 1,024 separated square patches, the local
component benchmark took approximately 101, 1,003 and 4,043 ms. Unioning each
equivalent run once reduced those measurements to 26, 45 and 102 ms. This is not
a measured full-map export speedup. Priority ordering and physical-receiver
boundaries remain separate; a 1,024-island regression checks coverage, gaps,
material bindings and input preservation.

The hill/river fixture now has 211 generated receivers instead of 214. All
non-receiver descriptor fields are identical after JSON serialization. Local
artifact `receiver-batch-review-uFKBKZ` retains both descriptors and the coverage
review: grouping height-plane coefficients at 1e-6 finds the same 21 groups,
with about 0.00001479 square units of footprint difference across two groups.
This reflects fixed-point intersection rounding, not exact geometry equality.

Native construction passes. All 166 focused editor/compiler checks and all 67
enabled native map-compilation integration tests pass, including polygon validity,
hill heights, river/ford restrictions and sampled actor routing. Five
data-dependent audits remain ignored in that integration run. A separate paired
native audit samples 194,481 positions, with zero coverage or material differences.
Its strict exact-height assertion fails at 3,485 positions: the maximum height
difference is 0.0000038146973 units. The paired report is retained; bit-identical
floating-point heights are not claimed. Type checking, focused lint, formatting,
the editor production build and the game build pass. No engine implementation
changed.

A subsequent bounds prefilter skips support/floor intersections when their
footprints are spatially disjoint. Cached support bounds and each floor's actual
clipped coverage bound retain touching cases for exact intersection. In
`receiver-support-bounds-w8C2Tn`, 64 and 256 separated 10-by-10 floors on a 20-unit
grid retain byte-identical compiled descriptors. Receiver construction drops
from approximately 74 to 31 ms and 599 to 44 ms respectively; total compilation
for the larger case drops from 1,361 to 785 ms. All 166 focused checks, type
checking, lint and formatting pass, with unchanged native fixtures. These are
component measurements, not a completed Wychford export timing.

## Gameplay compilation phase reporting (2026-10-08)

The export worker now reports compilation phases to the editor without settling
the pending export request. The saved-map audit logs those same phase boundaries
and records durations for each occurrence, retaining repeated phases during
best-effort retries. This exposes terrain, navigation, receiving-surface, mask,
connection and state-control work separately; it does not measure completion as
a percentage or change generated gameplay.

All eleven focused wall/worker checks pass. Worker checks cover keeping a request
pending through multiple progress messages, busy-request rejection, cancellation,
late messages, final-response delivery and callback failure. The wall descriptor
still exactly matches the native fixture when progress callbacks are enabled.
App type checking, focused lint and the production build pass. Browser interaction is not verified by
these transport tests.

A separate current Wychford diagnostic measured loading at 252 ms, pinned wall
calibration at 134 ms, and terrain generation at 35,155 ms with 17,924 surfaces
and 728 blockers. These component measurements do not establish full export
duration or identify the later compiler bottleneck.

## Generated receivers at native coordinate precision (2026-10-08)

The terrain integration fixture had drifted from current editor output and
contained a self-intersecting receiving polygon. It crashed native floor union.
Fresh compiler output removed that old defect but still contained two triangles
which collapse to lines in binary32, plus narrow contours which pinch or cross
after conversion.

Generated receivers now retain simple native contours unchanged, triangulate
only contours which become non-simple, and omit only pieces with zero native
area. Every repair/omission produces a warning. Triangulation retains the full
material partition's explicit height-plane anchors and material bindings.
Authored physical volumes are outside this repair path. The refreshed fixture
matches editor compilation and has 214 valid native receiver polygons, with
two triangulation warnings and three zero-area omission warnings. Explicit ring
closures remain intact. Local export `grid-terrain-review-2YQRkk` records the
final descriptor.

All 165 focused compiler/material/terrain/wall checks pass, including preservation of
representable subpixel floors and both sides of a pinched notch. The native
`asset_map_compilation` suite passes 67 tests, with five data-dependent audits
ignored. It validates all receiver polygons after native decoding, hill heights,
ford/river/crop restrictions and collision-connected route samples. The test
which intentionally changes receiver heights now removes stale explicit anchors
before deriving its baseline. No engine implementation change was needed.

Receiver self-intersection checks now sweep overlapping edge bounds instead of
comparing every edge pair. A local 10,000-vertex circle check dropped from about
500 ms to 9 ms while preserving its original polygon and all export fixtures.
This is a component benchmark, not a full-map export timing. The saved-map audit
now logs and records separate load, bounds, wall-calibration, compile and write
durations, and identifies the failing phase when an export raises an error.

## Split spline mask application boundaries (2026-10-08)

Cropping a spline source can split an open character or projectile application
line into disconnected fragments. Export previously omitted the complete mask.
The compiler now emits separate masks using the existing native format, retaining
each fragment's range without inventing a joining edge. Bitmap/alpha coverage and
receiver probes follow each fragment; view and obstacle rules occur only once per
repeat. Character and projectile fragment counts can differ without leaking an
absent boundary or closure flag into another record. Closed-boundary splits and
fully removed application boundaries retain explicit omission warnings.

The shared wall and mask geometry suites plus editor wall exports pass 36 tests,
including equivalence to separately authored fragments on curved, rising walls
at 0, 37 and 180 degrees.
The new synthetic mask fixture matches editor export, with all other descriptor
fields equal to the existing material-wall fixture. Three older wall fixtures
were refreshed with explicit receiving-plane anchors; removing only those new
fields reproduces their previous descriptors exactly. Native loading and 35,964
character/projectile point-query checks pass across three repeats, including
points outside each fragment and on both sides of its boundary. The broader
native suite passed 64 tests, failed three and skipped five. The failures are a
stale plane in an edited test receiver and two terrain-load polygon-union panics;
these are resolved by the receiver checks above. No runtime implementation changes
are required for the mask split.

## Upper castle stairs and independently owned receiving contacts (2026-10-06)

`stage-upper-castle-stair-contacts.mjs` authors four contact edges in three
neighbour assets. The shared upper-wall corner is solved against both stair
planes simultaneously; neither seam is invalidated by correcting the other.
The upper-wall edges have 150/205 and 187/205 mesh sample hits, with maximum
uncovered distances 0.691202 and 0.351948 units. The east-wall contact has 196/205
hits and a 0.068379-unit gap; the courtyard ground has 120/205 and 0.120680 units.
An explicit 0.7-unit authoring review bound records these discrepancies without
changing compiler/runtime connection tolerances. Access clearances belong only
to their receiving assets.

Surface-only stage `upper-castle-stair-contacts-qq6a2U` and full export
`nottingham-upper-stairs-connected-3QFSr6` reduce the original 22 route failures
to eighteen, all involving the short stair's three lower entrances. Its physical
receiver is stored separately. Stage `upper-castle-stair-contacts-gQWe0z` also
corrects vertices 24/25 of the existing courtyard receiver volume, retaining all
other geometry and bindings. That strip has 5/205 exact sample hits and maximum
mesh discrepancy 0.120486 units. Full descriptor
`nottingham-upper-stairs-physical-TDnCZC` passes all 92 stair routes and ten
control apply/reset checks.

`check-courtyard-west-stair-neighbours.mjs` now also checks the real upper-castle
four-asset assembly. `nottingham-castle-upper-stair-neighbour-placements-avGUOZ`
passes 352 native actor routes across two independent copies, four rotations and
two elevations, plus 96 missing/raised neighbour rejections. The earlier
surface-only assembly `nottingham-castle-upper-stair-neighbour-placements-7E3raA`
is superseded and was not actor-tested.

Final warning-bearing stage `upper-castle-stair-contacts-jPiTUe` publishes all
four definitions with backup `nottingham-upper-castle-publication-20261006`.
The flight warnings record 787/788 and 827/828 sampled mesh hits, with gaps below
0.034 units. Fresh published full-map geometry matches the native-tested candidate
after excluding only warnings; all eight published copied assemblies also match
their native-tested descriptors. All ten scenes reopen. Local anchor audit
`lift-anchor-support-4wkb1S` finds 19/53 unsupported stairs, with Nottingham 2/12;
ladders remain 1/8 and walls 9/9. Rendered integration remains open.

Next unpublished candidates: southwest wall stage `local-stair-seams-Dhvnyv`
has 833/833 flight mesh hits; `external-stair-landings-wxr3ej` passes 192 directed
actor routes and 64 missing/raised synthetic landing cases. North wall stage
`local-stair-seams-dtWG5K` has 1,055/1,055 flight mesh hits and needs a reviewed
3.745-unit correction to one entrance midpoint (authoring limit 3.75).
`external-stair-landings-zF1KJt` passes compilation and 64 missing/raised landing
checks; its actor routes have not yet been run. Both still need full-map and real
neighbour contact review before publication.

## Nottingham south curtain stair (2026-10-06)

Next candidate `local-stair-seams-jcgY1i` corrects both upper castle stairs,
`building-361-lift` and `building-363-lift`, with all seven entrances external.
Flight sampling finds 787/788 and 827/828 mesh hits, with maximum uncovered
distances 0.027475 and 0.033253 units. The external-landing fixture now accepts
multiple lifts and groups coplanar synthetic floors into shared receiving assets.
This prevents another fixture floor from masking removal of a shared landing.
`external-stair-landings-VP9eUm` passes 352 native actor routes and 96 missing or
raised landing cases across two copies, four rotations and two elevations.
The existing published single-stair fixture `external-stair-landings-vwFHdc`
remains descriptor-identical to `external-stair-landings-nFz16k`.

Full upper-stair candidate `nottingham-upper-stairs-level-0nCrrQ` still fails
22/92 routes (twenty in sector 153, two in sector 155). It remains unpublished.
The physical contact owners are `nottingham-castle-courtyard-ground`
(`building-366-walk-0`, edge 18), `nottingham-castle-upper-wall`
(`building-362-walk-0`, edges 7 and 8), and
`nottingham-castle-east-courtyard-wall` (`building-325-walk-0`, edge 39).
Their movement and physical receiving contours still need independent review.

`local-stair-seams-Nr8ORf` corrects `building-203-lift` in
`nottingham-south-curtain-wall-2`. Flight mesh sampling passes 1,448/1,448 points;
changed local upper landing edges have zero sampled uncovered distance.
`external-stair-landings-rKZwmd` passes 32 native actor routes and 32 missing or
raised landing rejections. `local-stair-placements-AFCR4n` passes sixteen routes
on editor terrain, plus eight raised-entrance rejections.

Full candidate `nottingham-south-curtain-level-zFZ3pm` initially fails both
lower-entrance directions. `stage-nottingham-south-house-contact.mjs` now
supports the independently owned terrain edge in `ground-section-0-0`:
`[1709,2246]` and `[1710,2209]` move to x=1709.306313459375 while preserving y.
All 205 strip samples have mesh support. Stage
`nottingham-south-curtain-wall-2-contact-uzEil8` produces full export
`nottingham-south-curtain-connected-hEJhYA`, passing all 92 stair routes and ten
control apply/reset checks. Publication backup is
`nottingham-south-curtain-publication-20261006`. Fresh published Nottingham
exactly equals the native-tested descriptor; all ten scenes reopen. Anchor audit
`lift-anchor-support-HZlP4H` now reports 21/53 unsupported stairs, Nottingham
4/12; ladders and walls remain 1/8 and 9/9. Rendered integration remains open.

## Nottingham prison-road platform ownership (2026-10-06)

`local-stair-seams-AvY4ss` corrects `building-483-lift` using a reviewed
2.76-unit upper landing edge selection bound. Mesh review finds 875/875 flight
sample hits and zero uncovered distance on the changed upper landing edges.
Initial independent-copy exports `external-stair-landings-rR57Kh` pass 32 routes,
but full Nottingham `nottingham-prison-road-level-vUuSIl` fails both lower
entrance routes. A fixed terrain exclusion still blocks the corrected seam.

`stage-nottingham-road-clearance.mjs` now supports this asset explicitly. It
seats the two foundation-clearance vertices by 0.683951/1.651489 units, adds
asset-owned upright-headroom collision to the low deck, and removes the reviewed
six-point exclusion and contour label from `ground-section-3-0`. Stage
`nottingham-road-clearance-FailWK` exports full-map candidate
`nottingham-prison-road-owned-0dSQoa`, which passes all 92 stair routes and ten
control apply/reset checks. Its asset-only `placement` stage generates
`external-stair-landings-nFz16k` (32 routes, 32 missing/raised landing rejections)
and `local-stair-placements-Fff3Nc` (sixteen routes, eight raised rejections).

The generalized ownership check generates `nottingham-road-ownership-ElyoOg`.
Native checks confirm eight blocked ground points beneath the deck and two
restored actor routes through the old footprint after removing the asset.
Sixteen additional fixture-ground crossings pass. Publication backup is
`nottingham-prison-road-publication-20261006`; fresh published Nottingham is
exactly equal to the native-tested descriptor and all ten scenes reopen.
Audit `lift-anchor-support-gsXXZw` finds 22/53 unsupported stairs, with Nottingham
5/12. Ladders remain 1/8 and walls 9/9. Full rendered integration remains open.

## Nottingham southwest prison stair (2026-10-06)

`local-stair-seams-s63sJS` corrects `building-468-lift` and its local upper
landing. `external-stair-landings-VwsvD0` passes 32 directed actor routes across
independent copies at four rotations and two elevations, plus 32 missing/raised
landing rejections. `local-stair-placements-gnaj83` passes sixteen routes on
editor terrain and eight raised-entrance rejections. Mesh review finds 829/829
flight sample hits and upper landing edge discrepancies below 0.029 units.

Full-map candidate `nottingham-prison-level-Dulxy5` initially fails both lower
entrance routes. The terrain-owned approach ends short of the physical seam.
The extended `stage-nottingham-south-house-contact.mjs` explicitly reviews this
second contact in `ground-section-6-0`; it moves the selected edge onto the stair
seam and checks 205/205 samples against the pinned terrain mesh. The stage
`nottingham-southwest-prison-contact-HUQenq` includes an upper landing mesh and
rendering warning. Full export `nottingham-prison-connected-pWeksU` passes all
92 stair routes and ten control apply/reset checks.

Publication backup is `nottingham-prison-publication-20261006`. Fresh published
Nottingham exactly equals the native-tested descriptor; all ten scenes reopen.
Audit `lift-anchor-support-T086X7` finds 23/53 unsupported stairs, with Nottingham
6/12. Ladders and walls remain 1/8 and 9/9. Rendered actor verification remains
open; these changes do not certify arbitrary placements or other assets.

## Nottingham south stair house and terrain contact (2026-10-06)

`local-stair-seams-SllNxf` corrects `nottingham-south-stair-house`'s
`building-030-lift` floor, local upper landing, door midpoints and owned
clearance. `external-stair-landings-lIHoHp` passes 32 directed native actor
routes across two independent copies, four rotations and two elevations;
all 32 missing/raised landing cases reject. `local-stair-placements-SQxDU7`
also passes sixteen routes after ordinary drops onto editor terrain, with eight
raised-entrance rejections. Mesh review finds 1,148/1,148 flight sample hits.
The upper landing's changed edges extend at most 0.309335 units beyond its mesh.

Full Nottingham `nottingham-south-house-level-SgbIZF` initially fails both
directions through the lower entrance. Its terrain boundary stops short of the
physical seam. `stage-nottingham-south-house-contact.mjs` inserts a local strip
into the independently owned terrain surface, moving the two contact endpoints
2.009983 and 1.376923 units. All 205 strip samples have ground-mesh support.
This is an asset-authoring correction; export does not fabricate a connection.
`nottingham-south-house-contact-QUrmfh` and full-map descriptor
`nottingham-south-house-connected-g5uPrX` pass all 92 stair routes and ten
control apply/reset checks. No runtime/compiler change was needed.

Final stage `nottingham-south-house-contact-hIPxEc` adds only the explicit upper
landing mesh/rendering warning. It is published with backup
`nottingham-south-house-publication-20261006`. Fresh published Nottingham
geometry equals the native-tested descriptor after excluding warnings, and all
ten saved scenes reopen with their pinned descriptors. Local anchor audit
`lift-anchor-support-wOv9oF` now finds 24/53 unsupported stairs, including
Nottingham 7/12. Ladders remain 1/8 and walls 9/9. Full rendered actor integration
and arbitrary-placement coverage remain open.

## Courtyard western stair and joined landing precision (2026-10-06)

`stage-courtyard-west-stair-contacts.mjs` corrects the independently owned ground
and western-wall contacts for `building-351-lift`. Ground edge vertices move
0.507/2.974 units; wall vertices move 1.059/1.017 units. Their sampled mesh
coverage is 98/205 and 188/205, with maximum gaps 0.851/0.102 units. The ground's
physical receiver has a separately reviewed contour: its contact strip has
69/205 supported samples and a maximum 0.418-unit gap. An explicit asset-local
volume replaces the implicit part volume while retaining its flags, material
bindings and query precedence. Access clearances affect only their owning assets.

Initial contact stage `nottingham-castle-west-stair-contacts-pHzkOw` fixes the
upper handoff but leaves the lower one blocked. Physical-receiver correction
`nottingham-castle-west-stair-contacts-N6l1YE` still fails until two precision
problems are resolved. The compiler previously matched each precise floor
contour against an entire joined motion region; none of the five individual
pieces matched. The captured `joined-landing-boundary.json` regression now
unions outer contours while retaining holes as separate movement obstacles,
and accepts the exact boundary only when it rounds to the emitted region.
Disconnected pieces still reject restoration.

Runtime landing collision also cast precise obstacle corners to single precision
before clipping. The captured `precise-landing-collision-corner.json` regression
fails before the fix and passes afterward in both directions. Collision contours
now retain double precision through clipping and seam cleanup. Existing tests
continue to preserve actual thin obstacles, changing barriers and unsupported gaps.

Full Nottingham `courtyard-west-stair-joined-Pfujvs` passes all 92 stair routes
and ten control checks. Independent copies in
`courtyard-west-stair-neighbour-placements-PP2Zj5` pass all 32 routes at four
rotations and two elevations; all 64 missing/raised-neighbour cases reject.
Final stage `nottingham-castle-west-stair-contacts-jhi1KS` adds only a warning
for the substantial stair mesh discrepancy: 423/929 sampled floor points have
support, with visible gaps up to 26.130 units. The saved refinement recipe
shortened the visible flight while retaining a vertical rise at its foot. That
mesh still needs repair; passing gameplay routes does not certify rendered parity.

The three definitions are published with backup
`nottingham-courtyard-stair-publication-20261006`. Published placements
`courtyard-west-stair-neighbour-placements-p6JIZK` and full Nottingham match
native-tested geometry after excluding only the added mesh warning. All ten
scenes reopen. Audit `lift-anchor-support-l9iTvR` finds 25/53 unsupported stairs,
including Nottingham 8/12; ladders and walls remain 1/8 and 9/9.

Fresh saved-map exports `saved-map-exports-eYNrRH` pass 288 stair and 84 climbing
routes with the compiler/runtime fixes. The correctly summed stair total is 288;
earlier reports saying 284 were arithmetic errors, not fewer checked routes.
The named historical batch reports were rechecked and their totals corrected
below. The editor suite passes 861 tests with two skipped; native stair navigation
passes 20 tests, and movement passes 210 with thirteen diagnostic tests ignored.
Native asset-map integration passes 66 tests with five diagnostics ignored;
all 71 saved-map control apply/reset checks pass. App typechecking, focused lint,
the editor build and the game build pass.

## Nottingham castle hall, west stair contact and collision precision (2026-10-06)

Next candidate: the separate courtyard western stair (`building-351-lift`) in
`local-stair-seams-BE5jMg` has two external landings. Synthetic independent
receivers in `external-stair-landings-tjMuG5` pass 32 directed routes and all
64 missing/raised-contact rejections. Full Nottingham
`nottingham-west-stair-351-level-HPsqj7` fails both directions through sector
155 at its receiving boundaries; the other 90 stair routes pass. Mesh review
finds 423/929 floor hits and a maximum uncovered distance of 26.130 units.
All 506 uncovered foot samples are visible at the initial rotation, and sampled
mesh treads span heights 78–150 while navigation reaches ground. The candidate
is unpublished: fixing only endpoints would leave a substantial mesh/floor
discrepancy. These diagnostics do not change the passing published baseline below.

Castle hall/watchtower stage `local-stair-seams-PIeiOT` corrects the
`building-500-lift` floor and both local contacts. Moved assembly
`local-stair-placements-i90nU8` passes sixteen directed routes and eight controls
at four rotations and two elevations. Full Nottingham
`nottingham-castle-hall-level-F7ULDb` passes 92 stair routes and ten controls.
Mesh review finds 704/789 flight hits, uncovered distances up to 5.885 units
and landing discrepancies up to 6.917 units. Sampled initial-state gaps are
occluded; alternate-state and rendered actor coverage remain unverified.

Warning-bearing stage `local-stair-seams-5hZtWL` is published with backup
`nottingham-castle-hall-publication-20261006`. Published placement exports
`local-stair-placements-rlzpmE` and full Nottingham equal the tested candidates
after excluding only the added draft warning. All ten saved scenes reopen.
Audit `lift-anchor-support-l1sB4l` records 27/53 unsupported stair definitions,
including Nottingham 10/12; ladders remain 1/8 and walls 9/9.

The unpublished west stair tower stage `local-stair-seams-QWsc3L` exposed a
floating-point clearance subtraction failure at 37 degrees. A captured
regression verifies that a narrowly scoped fixed-precision retry retains the
solid, cuts real openings and respects clearance ownership. The external
landing fixture now accepts intermediate collinear vertices on a single seam.
`external-stair-landings-TSxiQO` passes 32 native actor routes with 32
missing/raised-contact rejections.

Full Nottingham candidate `nottingham-west-tower-level-Js5R73` initially fails
native validation because a six-vertex collision strip self-intersects after
conversion to 32-bit coordinates. Collision partitioning retains its exact
pre-conversion coverage in four triangles and preserves each piece's state.
Fresh ten-scene exports `saved-map-exports-CCq9NH`, including the staged tower,
all pass native geometry construction and 84 climbing routes. Stair checks pass
286/288 routes: both failures are the tower's upper hall contact, whose receiving
boundary and access clearance still follow the previous stair contour.

`stage-nottingham-west-stair-contact.mjs` stages only the hall-owned receiving
edge and an asset-local access clearance on the corrected floor plane. Contact
stage `nottingham-west-stair-contact-XUFnSM` shifts the two receiving vertices by
1.196/1.375 units. Full Nottingham `nottingham-west-stair-connected-BI7vmJ`
passes all 92 stair routes and ten controls. The two independent real-asset
copies in `nottingham-west-stair-neighbour-placements-SDXNYW` pass 64 routes
and sixteen controls at four rotations and two elevations. All 32 missing or
raised hall cases reject. Unlike the earlier synthetic landing fixture, these
checks retain the actual hall's collision and its separate stair.

Mesh review records 180/205 supported receiver samples and a maximum 0.443-unit
gap. The tower flight has 618/623 supported samples, a maximum 0.307-unit gap,
and two sampled foot gaps visible at 180 degrees. Final stage
`nottingham-west-stair-contact-UsDjsZ` differs from the tested candidate only
in explicit draft review notes. Both definitions are published with backup
`nottingham-west-stair-publication-20261006`. Published placements
`nottingham-west-stair-neighbour-placements-cBcUTb` and full Nottingham equal
the native-tested geometry after excluding only those changed warnings. All
ten scenes reopen. Audit `lift-anchor-support-2W4NVY` records 26/53 unsupported
stairs, including Nottingham 9/12; ladders and walls remain 1/8 and 9/9.
Rendered integration and fresh browser ZIP verification remain outstanding.

Fresh published ten-scene exports `saved-map-exports-3xbSA4` pass all 288 stair
routes and 71 control apply/reset checks. The other nine descriptors exactly
match `saved-map-exports-CCq9NH`; Nottingham has no climbing definitions, so the
84 passing climbing routes remain unchanged by this publication.

The collinear-seam fixture change also preserves all eight earlier north-hall
exports exactly (`external-stair-landings-uujhMe` versus
`external-stair-landings-BNeL8E`).

Eighteen native JSON fixtures now include the compiler's precise ordinary-door
endpoints; their other fields are unchanged. The flattened-endpoint validation
test also updates its physical endpoint consistently with its projected point.
The native map-compilation suite passes 66 tests with five diagnostic tests
ignored. The editor suite passes 860 tests with two skipped; app typechecking,
focused lint and the production build pass.

## Published great hall with independent ramp and annex contacts (2026-10-06)

The full-scene passage candidate `lincoln-great-hall-passages-TqcwjI` initially
fails six of sixteen stair routes: the independently owned receiving boundaries
do not meet the corrected stair. `stage-lincoln-great-hall-contacts.mjs` stages
only the ramp's `building-288-walk-0` edge and the annex's
`building-198-walk-0` edge, verifies their translation-only authoring frames and
samples the affected strips against the pinned meshes.

Stage `lincoln-great-hall-contacts-Yg0Yg0` combines those edits with
`local-passage-seams-NYYqrN`. Ramp endpoint shifts are 3.901/5.188 units;
annex shifts are 1.817/1.897 units. Ramp strip coverage is 1/205 exact samples,
with its pre-existing edge reaching a 2.344-unit mesh gap; the corrected edge
reduces the maximum to 0.299 units. Annex strip coverage is 173/205, with a
0.473-unit maximum gap. Explicit draft warnings retain these limitations.
The stair's repeated mesh review retains 566/566 flight hits.

Full Lincoln `lincoln-great-hall-connected-4fbCiz` passes sixteen stair routes,
eight climbs and eleven control apply/reset checks. Real-neighbour assembly
`great-hall-neighbour-placements-nU0klO` places two independent copies at
0/37/90/180 degrees and heights 0/40: all 96 actor routes and 32 control checks
pass, while all 64 missing/raised-neighbour cases reject. This uses the actual
ramp and annex assets, not synthetic receiving floors.

Publication backup is `lincoln-great-hall-publication-20261006`; all three asset
definitions and Lincoln scene pins are updated. Published assembly exports
`great-hall-neighbour-placements-yr00AO` and full Lincoln exactly equal the
native-tested descriptors. All ten scenes reopen. Audit
`lift-anchor-support-sUVZfU` finds 28/53 unsupported stair definitions, with
Lincoln now 0/6; ladders and walls remain 1/8 and 9/9. Formatting checks pass.
Rendered integration and fresh browser ZIP verification remain outstanding.

## Physical ordinary-passage walks and directed receiver probes (2026-10-06)

Ordinary passage walks on physical stairs now retain world destinations on
both sides of the midpoint callback. The callback uses an authored midpoint
when it lies on the stair plane, including edge-on floors; legacy invertible
projections remain supported and invalid authored midpoints warn. A complete
actor-route regression reaches a fractional world endpoint exactly, and an
edge-on handoff regression preserves both endpoint identities.

Physical walking exposed two great-hall passage midpoints outside the corrected
floor. `stage-local-passage-seams.mjs` stages intersections of their approach
segments with the stair boundary, requiring one crossing and limiting movement
to 15 units. It creates candidates only: independent receiver, frame and mesh
review remains required. Stage `local-passage-seams-NYYqrN`, derived from
`local-stair-seams-xESqwA`, shifts the lower/upper midpoints 6.839930/11.933299
units while retaining all doors and inside/outside approaches.

The final four rotated failures came from receiver roundoff probes that advanced
X and Y by equal ULP steps. That direction can leave a sloped receiver even when
the exit waypoint is inside it. Probes now follow the actual approach vector,
bounded by four local f32 steps; they do not jump to the outside waypoint.
A focused sloped-edge callback regression covers this distinction.

Independent great-hall fixture `external-stair-landings-5btVbp` now passes all
96/96 actor routes and retains all 64 missing/raised landing rejections. The
candidate remains unpublished pending the real neighbouring contacts in full
Lincoln, mesh review and rendered verification. Fresh saved-scene batch
`saved-map-exports-gibq1F` compiles all ten maps and passes 288/288 stair routes
and 84/84 climbing routes. These descriptors include the newly exported
ordinary-passage world endpoints. All 71 control apply/reset checks also pass.
All 195 movement tests pass, with thirteen ignored; focused ordinary-passage
tests, formatting checks and the game build pass.

## Preserved ordinary passage world endpoints (2026-10-06)

Standalone passages now export optional `world_endpoints` with their placed
inside, middle and outside coordinates. Integer waypoints remain the compiled
spatial identities. Loading checks finite coordinates and agreement with those
identities, retains the endpoints, and derives precise runtime projections.
Physical stair gate approaches select the endpoint for their current side
directly. Older exports without the field remain supported. Replay schema 63
records the added runtime door state.

Fresh independent great-hall fixture `external-stair-landings-rgszot` passes
74/96 routes and retains all 64 missing/raised landing rejections. This is a
net improvement over 70/96, but it also exposes two position-consistency failures
per elevation at zero degrees, where floor support and height are correct.
The 90-degree cases improve from four passing routes per export to eight;
receiver failures remain. The candidate is unpublished. Ordinary passage
animation/handoff must retain consistent physical positions throughout, not
only during gate approach; remaining receiving contacts still require review.

The focused movement regression uses a fractional world goal while supplying a
rounded map destination and passes in both directions. Loader round-trip and
projected-identity rejection checks pass. Validation also passes 126 compiler
tests, application typecheck, focused lint/formatting, 192 movement tests,
40 level-loading tests, 29 replay tests and 85 level-data tests. The existing
saved-map descriptors retain all 288 stair and 84 climbing routes; game and
editor production builds pass. Ignored tests remain outside those counts.
These results do not certify full-map rendering
or baked ZIP parity.

## Ordinary gate approaches from physical stairs (2026-10-06)

Approaches to ordinary gates now retain gate identity in both directions when
leaving a physical stair. Their movement dispatch resolves the projected gate
destination onto the current physical plane instead of using projected floor
extraction. The focused regression covers ordinary gates and point destinations
in both directions, including gate requests without an explicit layer or sector.

The unpublished great-hall assembly `external-stair-landings-VQ7JOS` improves
from 63/96 to 70/96 passing routes. All 0- and 37-degree copies pass at both
elevations; 90- and 180-degree failures remain. Diagnostics now distinguish
height mismatch from unsupported footprint and include the current order queue.
The candidate remains unpublished.

Validation: 192 movement tests pass (thirteen ignored), the existing
`saved-map-exports-rdmm0t` descriptors retain 288 stair and 84 climbing routes,
and `lincoln-north-hall-level-KSbGJF` retains sixteen stair routes. Formatting
checks and the game build pass. These checks do not constitute fresh ZIP or
rendered verification.

Follow-up diagnosis compares the placed authored ordinary-door endpoints against
the compiled stair plane at 0/37/90/180 degrees. Both stair-side endpoints lie
on the plane (height error below 5e-13). At 90 degrees, inverting their rounded
map waypoints displaces them by 41.754 and 106.710 ground units; inverting the
unrounded projection has error below 5e-11. Thus the authored approach heights
are consistent here, but integer door waypoints cannot recover their physical
positions. Ordinary passages need preserved world endpoints through compilation
and runtime handoff/movement, including edge-on projections. Increasing support
tolerances or extending the stair footprint would hide the error.

## Ordinary passage handoff to physical stairs (2026-10-06)

Ordinary passage doors can enter a physical stair sector without belonging to
the lift's endpoint list. Their callbacks previously retained the previous
floor plane: a focused regression entered the fixture stair at height zero
instead of its physical height. The callback now installs the target stair
plane when its screen projection is invertible and restores the receiving
plane when an ordinary door leaves a physical stair. The regression covers both
entry and exit and fails before the fix. Edge-on ordinary doors still need
authored world endpoints; the unresolved handoff warns explicitly.

The unpublished great-hall copied assembly `external-stair-landings-VQ7JOS`
improves from 32/96 to 63/96 passing routes. Remaining rotated movement/support
and landing-contact failures still prevent publication. The complete movement
suite passes 192 tests, with thirteen ignored. Existing saved-map batch
`saved-map-exports-rdmm0t` retains all 288 stair and 84 climbing routes; the newer
published Lincoln descriptor `lincoln-north-hall-level-KSbGJF` also retains all
sixteen stair routes. The final focused handoff test, formatting and game build
pass. These are runtime regression checks against existing
descriptors, not fresh baked ZIP or rendered verification.

## Great-hall sloped external landing review (2026-10-06, unpublished)

The remaining Lincoln great-hall stair receives its lower entrance from
`lincoln-hall-approach-ramp/building-288-walk-0`, not a flat floor at its outside
waypoint height. The ramp plane transformed into the stair asset's frame is
`[0.22021854541102542,-0.2523393908114075,427.78412526426655]`; the upper landing
belongs to `lincoln-keep-annex/building-198-walk-0` at height 550.00104.
The flat assumption rejects an excessive midpoint correction.

`stage-local-stair-seams.mjs` now accepts explicit reviewed
`--external-plane=DOOR=A,B,C` arguments for selected external doors, validates
the outside waypoint against each plane and records the plane in its review.
`check-external-stair-landings.mjs` constructs the independent receiving fixtures
on those planes, including their varying vertex heights and intersection edge.
Existing stages without recorded planes retain flat fixtures. North-hall
regression `external-stair-landings-5QWrq8` exactly matches the previously
native-tested flat descriptors, with all 64 negative connection checks retained.
Both scripts pass formatting checks.

Great-hall stage `local-stair-seams-xESqwA` has mesh support for all 566 sampled
flight points. Independent copies in `external-stair-landings-VQ7JOS` pass
32/96 directed routes and reject 64 missing/raised landing cases. The extra
routes arise from ordinary hall passage doors attached to the stair sector;
64 fail physical support or receiver checks. Full Lincoln
`lincoln-great-hall-level-sac1zs` also fails: both external landing boundaries
need correction and ordinary passage entry can lose physical support. The
candidate is unpublished. Fixing the door/physical-floor handoff and reviewing
both independently owned contacts remain necessary; omitting those doors is
not an acceptable substitute.

## Lincoln north-hall stair with independent landing owners (2026-10-06)

Stage `local-stair-seams-58q3n8` corrects the stair floor and both external
endpoint seams. The single-flat-ground fixture correctly omits the stair because
its upper landing is missing. Independent receiving assets in
`external-stair-landings-QVIYHf` instead exercise two stair copies at four
rotations and elevations 0/40: all 32 directed native routes pass and 64 missing
or raised landing cases reject. All 598 flight samples have mesh coverage.

The full scene initially fails both directions because neither receiving
boundary reaches its corrected physical entrance
(`lincoln-north-hall-level-A8uv95`). Ground-contact stage
`lincoln-stair-ground-contact-CAjnA6` updates the terrain and north-bailey
plateau's matching blocker, shifting endpoints 0.126/0.949 units. All 205
sampled contact points have mesh support. `stage-lincoln-north-hall-platform.mjs`
then adjusts only the stair-width portion of the independent west curtain's
upper receiving edge. The remaining long edge is retained. Upper shifts are
0.854/0.915 units; all 205 mesh samples pass. Combined stage is
`lincoln-north-hall-platform-Dc3cut`.

Full Lincoln `lincoln-north-hall-level-KSbGJF` passes native construction,
sixteen stair routes, eight climbing routes and eleven control apply/reset
checks. Publication backup is `lincoln-north-hall-publication-20261006`; all four
asset definitions and Lincoln scene pins are updated. Published independent
placements `external-stair-landings-BNeL8E` and full Lincoln exactly match the
tested descriptors. All ten scenes reopen. Refreshed local support audit
`lift-anchor-support-aSmJpO` finds 29/53 unsupported stair definitions, including
1/6 in Lincoln; ladders and walls remain 1/8 and 9/9. These actor-loop and mesh
checks do not establish complete rendered parity.

## Lincoln east curtain stair and north-bailey contact (2026-10-06)

Stair seam review `local-stair-seams-tmUCYD` corrects `building-150-lift` in
`lincoln-east-curtain-wall-middle`, using local sloped landing edges and an
external lower entrance with new-drop ground height 220.001. Placements
`local-stair-placements-3FRu21` pass sixteen directed routes at four rotations
and elevations 0/40; eight raised approaches reject. The asset's separate
climbable wall is not corrected by this stage.

Full Lincoln initially fails the two lower-entrance directions because its
terrain receiver does not reach the stair (`lincoln-east-curtain-level-k6rJdN`).
The generalized `stage-lincoln-stair-ground-contact.mjs` now reviews both south
and east contacts. East terrain edge `[2691,981]`–`[2717,999]` and the matching
north-bailey plateau blocker move less than 0.79 units to the stair plane.
The pinned mesh supports 200/205 exact contact samples; the existing end-strip
discrepancy decreases from 0.416 to 0.040 units. This remains an explicit draft
authoring bound, not a runtime connection tolerance.

Combined stage `lincoln-stair-ground-contact-PGckIX` produces full Lincoln
`lincoln-east-curtain-level-RnmGP7`, passing native construction, sixteen stair
routes, eight climbing routes and eleven control apply/reset checks. Mesh
review finds only 537/566 stair-flight samples supported, with a 10.006-unit
maximum uncovered margin; none of the reviewed upper landing-edge samples
has mesh support at the authored height. Final stair stage
`local-stair-seams-0Aero6` and combined stage `lincoln-stair-ground-contact-rrBuIo`
retain tested geometry and add the explicit visual limitation warning.

Publication backup is `lincoln-east-curtain-publication-20261006`; the stair,
terrain, plateau and Lincoln pins are updated. Published placements
`local-stair-placements-JcNCW5` and full Lincoln match tested geometry except for
the added draft warning. All ten scenes reopen. Audit
`lift-anchor-support-Vqbiuj` finds 30/53 unsupported stair definitions, including
2/6 in Lincoln; ladders and walls remain 1/8 and 9/9. Rendered actor integration
and the documented geometry discrepancies remain unresolved.

## Lincoln south-wall stair and plateau contact (2026-10-06)

The lower approach intersects the landing plane 3.319 units from its authored
midpoint. The upper landing's nearest edge needs a 3.428-unit search bound.
Review stage `local-stair-seams-1ucLQc` uses limits 3.32 and 3.43, local sloped
landing edges and an external lower entrance, with new-drop ground height
220.001. All sixteen directed routes pass at four rotations and elevations
0/40 in `local-stair-placements-X9p0x6`; eight raised approaches reject.
Mesh review finds 588/591 flight samples supported, maximum uncovered margin
0.072 units. Upper landing-edge discrepancies reach 1.786 units. Final stair
stage `local-stair-seams-DBLgSH` adds the explicit draft warning without changing
that tested geometry.

Full Lincoln initially fails both directions through the lower entrance
(`lincoln-south-stair-level-pHuqLv`): the terrain receiver does not reach the
physical door. `stage-lincoln-stair-ground-contact.mjs` corrects the corresponding
boundary in `lincoln-terrain/ground-section-1-0` and its independent blocker in
the inner-bailey plateau asset. Endpoint shifts are 0.688 and 2.361 units. The
pinned plateau mesh supports 201/205 exact samples; four end-strip samples lie
at most 0.373 units outside it. An explicit 0.375-unit draft authoring bound and
warning record this discrepancy, without relaxing runtime connection checks.
Combined stage is `lincoln-south-stair-contact-A9LQka`.

Full Lincoln `lincoln-south-stair-level-AXhdI8` passes native construction, all
sixteen stair routes and eleven control apply/reset checks. Publication backup
is `lincoln-south-stair-publication-20261006`, updating all three asset definitions
and the Lincoln scene pins. Published placements `local-stair-placements-ColpKP`
match tested geometry except for the added warning; full Lincoln exactly equals
the tested combined descriptor. All ten scenes reopen. Anchor audit
`lift-anchor-support-mrRESk` now finds 31/53 unsupported stair definitions,
including 3/6 in Lincoln; ladders and walls remain 1/8 and 9/9. This is still
initial-state traversal evidence, not complete rendered or arbitrary-placement
parity. No compiler or runtime code changed.

## Lincoln west slate tower stair seams (2026-10-06)

Both tower flights now have corrected local floor/landing seams, with the lower
external entrance defining the 220.001 new-drop ground height. Review stage
`local-stair-seams-arXwj0` passes 32 directed native routes in
`local-stair-placements-bV2Cmd`, covering four rotations and elevations 0/40.
All eight raised external approaches reject. Full Lincoln
`lincoln-west-slate-level-Fuf4Uh` passes native construction, all sixteen stair
routes and eleven control apply/reset checks. The isolated tower exports omit
controls that need external neighbours; the control harness correctly rejects
that zero-control batch, so only the full-scene result establishes control
coverage. This publication corrects the stairs, not every external attachment.

Mesh review finds 791/794 lower-flight samples supported (maximum uncovered
margin 0.057 units), and 660/690 upper-flight samples (1.131 units). Landing-edge
discrepancies reach 0.345 units. Final stage `local-stair-seams-ILOSlg` adds an
explicit draft warning for those discrepancies and unverified rendered actor
integration; its gameplay otherwise exactly matches the tested stage.

Publication backup is `lincoln-west-slate-publication-20261006`, with the Lincoln
scene pin refreshed. Published placements `local-stair-placements-P3nQhe` and
full Lincoln match their tested descriptors except for the added draft warning.
All ten scenes reopen. Refreshed audit `lift-anchor-support-wT4nMf` finds 32/53
unsupported stair definitions, including 4/6 in Lincoln; the ladder/wall totals
remain 1/8 and 9/9. No compiler or runtime code changes were needed.

## West-moat tower and obstacles formed by joined floors (2026-10-06)

The tower's stair needs corrected floor/landing seams and collision clearance
for the newly extended portions of its own landings. Base review
`local-stair-seams-bk6KfR` uses local sloped landing edges with a 2.05-unit
landing shift limit. `stage-landing-extension-clearances.mjs` derives two
clearances from the reviewed after-minus-before landing coverage, preserving
holes and requiring unchanged landing planes. It does not clear whole floors
or collision belonging to other assets. Final stage is
`landing-extension-clearances-7y3c3n`.

At 180 degrees, joining two floor pieces creates an enclosed movement obstacle
that neither piece had individually. The compiler now recovers its precise
boundary from the union of the placed floor coverage and their holes. Recovery
still requires the same rounded footprint; ambiguous existing contours retain
their fallback. Regression tests cover the fractional landing contact, precise
holes and ambiguous-hole collision.

Final placements `local-stair-placements-B2QkGa` pass sixteen directed native
actor routes and eight control apply/reset checks at four rotations and two
elevations. All eight emit physical navigation. Mesh review in
`local-stair-seams-LHaGP8` finds 607/746 floor samples supported, with a maximum
uncovered margin of 3.886 units and landing-edge discrepancies up to 0.334 units.
The draft explicitly retains these limitations and unverified rendered actor
integration. Editor validation passes 857 tests, with two skipped, app
typechecking, focused lint, formatting and the production build.

Fresh staged batch `saved-map-exports-rdmm0t` passes native construction for all
ten maps, 288 stair routes, 84 climbing routes and 71 control apply/reset checks.
Publication backup is `west-moat-publication-20261006`; Leicester and Wychford
pins are refreshed. All eight published placement exports
(`local-stair-placements-BQ3vU8`) and the full Leicester descriptor exactly match
their native-tested candidates. All ten saved scenes reopen. Refreshed local
anchor audit `lift-anchor-support-nVNL7S` finds 34/53 unsupported stair
definitions, 1/8 ladders and 9/9 climbable walls. All eight Leicester stairs now
pass that local support audit; this does not establish arbitrary-placement or
rendered parity.

## Nottingham road stair ownership and precise terrain obstacles (2026-10-06)

The road stair's midpoints were 0.136/0.168 units off their receiving heights.
Stage `local-stair-seams-VXsoEy` corrects both seams and their upper platform
contact, sets the new-drop foundation height to 0.7617074580551671, and passes
sixteen routes at four rotations and two elevations. Full Nottingham still
failed because its terrain retained a permanent exclusion for the platform.

`stage-nottingham-road-clearance.mjs` authors a 0.625/0.583-unit correction to
the asset's foundation clearance and moves low-deck movement collision into
asset-local volumes with 80 units of upright headroom. The volume shapes reuse
the local timber bounds without adding opaque or pickable copies. The reviewed
terrain hole and its contour label are removed together. Final stage is
`nottingham-road-clearance-rLkKPJ`; its `placement` subdirectory tests the asset
independently of the corrected terrain.

Preserved movement boundaries previously discarded the fractional contours of
added obstacles. They now retain complete source contours; normal emission still
requires a unique exact contour with identical grid rounding. Holed or ambiguous
partitions retain their existing fallback. The focused regression and all 855
editor tests pass, with two skipped; app typechecking, lint, formatting and the
production build pass.

Current placements `local-stair-placements-wzAusU` retain sixteen successful
stair routes and reject eight raised approaches. Full Nottingham
`nottingham-road-level-wlmiZo` passes all 92 routes. Ownership audit
`nottingham-road-ownership-utNYVI` passes eight blocked under-deck points and
eighteen directed ground routes, including both directions through the former
platform footprint with the asset removed. The native route harness now supports
explicitly declared same-receiver routes while retaining receiver, height,
arrival and multi-tick movement assertions.

Mesh review finds 753/829 floor samples supported, a maximum uncovered margin
of 2.504 units, and at most 0.493 units of floor adjustment. Upper-contact edge
discrepancies are below 0.048 units. The draft warns about incomplete visible
coverage and unverified rendered actor integration.

Fresh staged batch `nottingham-road-all-maps-U3N6zl` passes native construction
for all ten maps, all 288 stair routes, 84 climbing routes and 71 control
apply/reset checks. Another 191 movement tests pass (thirteen ignored), and the
game build passes. Publication backup is `nottingham-road-publication-20261006`;
the road asset, terrain definition and Nottingham scene pins are updated.
All eight published placement exports (`local-stair-placements-db7spY`) and the
full Nottingham export exactly equal the tested candidates. All ten scenes reopen.

## Published southwest turret and partial obstacle edge recovery (2026-10-06)

The southwest turret's lower ground obstacle could not recover its entire exact
contour: one rounded edge spans several short source edges, and the blocked
coverage contains tiny holes. Recovery now has a fallback that follows uniquely
identified edge interiors between their endpoint rounding cells, retaining grid
corners and ambiguous edges. Union with the authored solid already covered by
grid collision prevents removal of genuine blockers. A single-contour result
must still round to exactly the emitted movement obstacle. Unrelated blocked
regions are excluded from ownership selection; overlapping candidates reject.
The captured raised-ground fixture checks both elevations, the cleared contact,
retained solid coverage and rejection of ambiguous ownership.

`local-stair-placements-WtogDs` passes all sixteen directed actor routes at
0/37/90/180 degrees and elevations 0/40; its eight raised-ground cases reject.
Full Leicester (`southwest-turret-level-wIIbVx`) passes sixteen stairs, 22 climbs
and twelve control apply/reset checks. Mesh review (`local-stair-seams-w1SSea`)
finds 694/695 floor samples supported, with a 0.032486-unit uncovered margin.
Complete rendered actor integration remains unverified and is explicitly warned.

Final stage `local-stair-seams-kmOb5W` differs from the tested stage only in draft
warnings. Publication backup is `southwest-turret-publication-20261006`; Leicester
and Wychford pins are refreshed. All eight published exports
(`local-stair-placements-HKqvOs`) and the full Leicester export equal their tested
geometry after excluding warning text. All ten scenes reopen. Anchor audit
`lift-anchor-support-iMwsaD` now finds 35/53 unsupported stair definitions,
including one of Leicester's eight; ladder/wall counts remain 1/8 and 9/9.

Validation: 854 editor tests pass, two skipped; app typechecking, focused lint,
formatting and production build pass. The native game build from the preceding
runtime change is current. This publication does not resolve the separate
Nottingham height mismatches or certify other arbitrary placements.
Fresh all-map batch `saved-map-exports-vD9Ih4` passes native construction for
all ten maps, all 71 control apply/reset checks and 84 climbing routes. Its stair
audit retains 286/288 passing routes, with only Nottingham sector 156 failing
in both directions. No additional stair failures appear with the compiler change.

## Precise landing contours and remaining route failures (2026-10-06)

Landing boundaries and holes now retain double precision through binding and
foot-support routing. The captured `precise-landing-floor.json` regression has a
valid exact contour that becomes self-intersecting when reduced to single
precision; binding and seam-to-interior routing now succeed. Serialized bound
landing coordinates change representation, so replay schema advances to 62.

Precise clipping also exposed a rounding strip attached to a real wall cap.
Cleanup subtracts only the bounded strip along a matching stair/solid edge,
preserving the wall cap, obstacle identity and standalone thin solids. Focused
tests cover those distinctions. Full Leicester (`great-keep-level-JTtAxp`)
retains all sixteen routes after this correction.

Southwest placements `local-stair-placements-ld1PeN` now pass twelve of sixteen
routes, up from eight. The four remaining failures are zero-degree placements
at both elevations. The lower landing still has an integer-only obstacle whose
rounded triangle overlaps the exact seam; precise contour recovery remains open.
No candidate asset is published by this runtime change.

The saved-scene batch `saved-map-exports-pURUNj` passes 286/288 stair routes and
84/84 climb routes. Both stair failures are Nottingham sector 156, doors 124/125.
Its physical midpoints are at heights 0.135946 and 29.832588 while the landing
heights are 0 and 30.001001. The existing height guard rejects both before any
changed contour code executes; relaxing it would invent support across a gap.
The complete batch was rerun after the Nottingham-only trace to retain all ten
maps in its route report.

Validation: nineteen stair-navigation tests, 191 movement tests (thirteen
ignored), 29 replay tests and all four changing-climb tests pass. The latter
include 72 reopening checks. Formatting, diff whitespace checks and the game
build pass.

## Published great keep stair and fractional receivers (2026-10-06)

`local-stair-seams-78dS02` corrects the great keep stair's end heights and sets
new-drop ground height to 50.000920131548675. Its lower approach midpoint moves
5.128 units to the intersection of its authored approach and the actual landing
height; the stair boundary moves at most 1.116 units. Mesh review finds 676/677
floor samples supported, with the remaining sample 0.032 units from the edge.
The existing upper platform already covers its seam.

Eight independent placements (`local-stair-placements-YPfgAw`) pass sixteen
stair routes and sixteen control apply/reset checks, with eight raised-ground
connections rejected. Full Leicester (`great-keep-level-JTtAxp`) passes sixteen
stairs, 22 climbs and twelve controls. Publication backup is
`great-keep-publication-20261006`; Leicester and Wychford pins are refreshed.
Published placement descriptors (`local-stair-placements-ANZYN3`) and the full
Leicester descriptor exactly equal their tested candidates. All ten scenes reopen.

The southwest turret candidate exposed a separate compilation error: a valid
fractional receiving triangle was rejected by the 0.5 minimum intended for
integer-grid motion polygons. Exact receiving boundaries now use the existing
1e-8 physical-area threshold. A regression compiles the small triangle while
retaining its valid integer-grid area. All 853 editor tests pass, two skipped;
app typechecking, focused lint and the production build pass.

Southwest candidate `local-stair-seams-ifICYn`, with ground reference
50.00641218574032, now exports all eight placements
(`local-stair-placements-ld1PeN`). Eight of sixteen actor routes still fail at
0/90 degrees. The zero-degree loader rejects the upper landing as degenerate
or self-intersecting; the candidate remains unpublished. Its mesh review finds
694/695 flight samples supported with a maximum uncovered distance of 0.033.
This remains an unresolved landing-geometry problem, not a completed asset.
Native construction passes all eight exported descriptors, including the small
receiver contours; this does not establish the failing actor traversal.

## Published northwest tower stair and receiving contact (2026-10-06)

Staging `leicester-northwest-tower/building-330-lift` with an external lower
landing and `placementGroundHeight=140.00091880252668` seats new drops at their
actual approach. Candidate `local-stair-seams-3Oq8sz` passes sixteen directed
stair routes and eight control apply/reset checks in `local-stair-placements-QoWyNE`;
all eight raised-ground cases reject. The upper landing edges have 41/41 sampled
mesh support. The flight has only 111/674 mesh hits, with uncovered distances
up to 7.865 units; final stage `local-stair-seams-7YHjQR` retains this draft warning.

The full scene exposed a polygon-clipping sweep-tree failure when comparing
near-coincident receiving contours. That comparison now uses the compiler's
existing fixed-point boolean operation. A captured contour-pair regression
preserves real seam differences while treating coordinate roundoff as identical.
All 852 editor tests pass, two skipped; app typechecking, focused lint and the
production build also pass.

Full-scene candidate `northwest-tower-level-WMGd6Q` still failed both tower
routes because the keep-owned platform did not meet the lower seam.
`refinement/stage-northwest-tower-contact.mjs` stages that edge in the keep's
own local frame. Its maximum movement is 0.879 units. Of 205 sampled points,
188 have mesh coverage within 0.1 height units; the rest lie at most 0.195 units
from the visible platform. An explicit 0.25-unit authoring review bound accepts
this discrepancy without changing runtime connection tolerances.

Final contact stage `northwest-tower-contact-n9j7Tf` adds the visual warning to
the tested geometry. Full Leicester `northwest-tower-level-hjYBcH` passes all
sixteen stairs, 22 climbs and twelve controls. Publication backup is
`northwest-tower-publication-20261006`; Leicester and Wychford pins are refreshed.
The published full Leicester and eight independent tower placements
(`local-stair-placements-WJmb0I`) reproduce the tested descriptors apart from
the added warnings. All ten scenes reopen. Local-anchor audit
`lift-anchor-support-Sf06eA` now finds 37/53 unsupported stairs, 1/8 ladders
and 9/9 walls. Complete rendered traversal remains open.

## Published York scaffold ladder (2026-10-06)

The upper platform notch required a reviewed landing shift limit of 7.1 units;
the default seam tool correctly rejected unrelated nearby edges. Stage
`local-stair-seams-CCthzT` seats the floor and upper platform but passes only
four of sixteen moved climbs. Both landings bind, but the off-center authored
waypoints leave insufficient actor foot support on the narrow flight.

`refinement/stage-york-scaffold-ladder.mjs` places the middle and inside waypoints
on the corrected flight centerline, preserving inside heights and outside
anchors. Both corrections are less than two game units relative to the seam
stage. The centered candidate `york-centered-ladder-8uILVB` passes all sixteen
complete-animation routes at 0/37/90/180 degrees and elevations 0/40
(`local-stair-placements-eTF39l`). Final stage `york-scaffold-ladder-bTzr6k` adds
the explicit mesh warning; its eight exports (`local-stair-placements-26tsEe`)
are identical except for that warning.

Mesh review samples 718/744 flight points on the visible mesh, with uncovered
edges up to 0.711 units away. Reviewed upper landing edges have discrepancies
up to 0.537 units; nearby assembly parts support portions outside the platform
part itself. These remain draft visual limitations, not rendered certification.
The full York candidate (`york-scaffold-level-COaN6u`) passes all forty climb
routes, 130 stair routes and six control apply/reset checks.

Publication backup is `york-scaffold-publication-20261006`; only York's scene
pin changes. All eight published placement exports
(`local-stair-placements-l9cFwe`) exactly equal the final staged descriptors,
and the published full York descriptor equals the native-tested candidate.
All ten saved scenes reopen. Audit `lift-anchor-support-Xdvy5w` now finds one
unsupported ladder out of eight, alongside 38/53 stairs and 9/9 walls.

Reproduce the authoring stages before publication:

```sh
node refinement/stage-local-stair-seams.mjs york-bridge-square-scaffolded-corner-house building-133-lift --climb-seams --local-landing-edges --sloped-landings --landing-shift-limit=7.1
node refinement/stage-york-scaffold-ladder.mjs <seam-stage>
```

## Generated physical-floor hole cleanup (2026-10-06)

A rotated floor with retraced boundary vertices produced a spurious triangular
hole during polygon union. Its vertices differ from collinearity only within
floating-point coordinate precision; subsequent export-frame clipping rejected
the floor. Generated union/clipping holes now discard only contours collinear
within eight double-precision coordinate units of roundoff. Authored polygons
still undergo normal validation, and a regression preserves a genuine 0.001-unit
wide hole and its permanent collision identity through frame clipping.

All 851 editor tests pass (two skipped), along with app typechecking, focused
lint/format checks and the production build. The diagnostic three-door asset now
emits physical navigation at all eight placements in
`local-stair-placements-TltOFJ`. All 48 native actor routes still fail because
landing support remains unresolved; the twelve formerly projected routes now
expose that physical failure too. This is a contour-compilation correction, not
a successful traversal candidate, and the asset remains unpublished.
The refreshed native integration binary constructs all eight exported geometries,
including their motion areas, sight obstacles, doors, grids and elevation
boundaries. Construction does not certify the failed actor routes.

## Permanent landing holes and height validation (2026-10-06)

Shared-edge validation previously treated permanently blocked platform holes as
usable landing contacts. It now subtracts permanent collision from the candidate
contact segments, preserving all collision identities and live-state handling.
State-dependent blockers cannot conceal incompatible floor heights. A synthetic
stepped flight verifies successful binding with a permanent hole and rejection
with partial coverage or a removable blocker. All eighteen stair-navigation tests
and all four changing-climb tests pass, including 72 reopening checks.
The existing ten-map descriptor batch `saved-map-exports-pURUNj` retains all
84 passing climb routes. Matching-height seams retain their existing rounding
handling; clipping those contacts as well initially regressed two Leicester
routes, and narrowing clipping to height-mismatched contacts restores both.
The game build and Cargo formatting check pass.

The diagnostic three-door candidate in `local-stair-placements-jeBV3z` still
fails 36 of 48 routes. The first blocked contact is correctly excluded, but
another exposed ground-height edge remains adjacent to the upper receiver.
The zero-degree trace locates it at (1668.6228, 1821.5415), with a height
difference of about -278.001. No candidate asset was published; the authoring
geometry and remaining multi-entrance support still need correction.

## Fractional ladder exit animation receiver fix (2026-10-06)

The 37-degree receiver mismatch occurred after the physical door callback seated
the actor at its exact seam. The exit animation still targeted the integer door
midpoint, moving the actor outside the receiver before the next walk. Physical
door translation now supplies the exact projected middle to transition animations
as well as climbing orders, retaining normal transition membership effects.

All 8/8 routes in `hole-edge-ladder-placements-hejrdt` now pass. The committed
`physical-ladder-fractional-seam.level.json` reproduces the failing 37-degree
case; its complete-animation native regression passes both directions, and the
editor test proves it equals a fresh compilation. All four changing-climb tests
pass, including 72 reopening checks; all 46 door-pass tests pass. The existing
ten-map descriptor batch `saved-map-exports-pURUNj` retains 84/84 passing climbs
with the updated runtime. This batch predates the west-treehouse publication;
it is runtime regression evidence, not a fresh full-library bake.
The game build and Cargo formatting check pass. The new editor fixture equality
test, focused lint and formatting pass. Rechecking the complex three-door
candidate still yields 36 failed physical routes, so this fix does not establish
its landing support or mesh alignment.

## Platform-hole ladder seams and three-door candidate (2026-10-06)

Physical ladder eligibility now permits its middle waypoint on a platform-hole
edge within the existing clipping precision. The outside waypoint still needs
ordinary support; the regression rejects a seam 0.01 units inside the hole.
Native fixture `hole-edge-ladder-nnxoT9` passes both complete-animation routes.
Four rotations (`hole-edge-ladder-placements-hejrdt`) pass 7/8 routes. The failing
37-degree receiver mismatch is identical with the hole removed in
`ladder-no-hole-baseline-RF6k1r`; this remains a separate traversal problem.
Editor validation passes 849 tests, two skipped; app typecheck, focused lint,
formatting and production build pass.

Three-door candidate `three-door-oak-7osA0K` preserves the existing outer platform
and connection sockets, replacing only mesh-supported portions of its hole.
The mesh closing result has no interior ring, but its exterior is indented:
it must not be interpreted as a filled platform disk. Remaining unsupported
coverage stays excluded. Eight placements retain all three entrances and reject
raised ground contacts. Native checks (`local-stair-placements-ASFIRA`) pass
36/48 routes; all twelve 90-degree routes fail in projected navigation.
The left upper seam remains 2.436 units inside the reconstructed opening.

A separate diagnostic seam strip (`three-door-oak-oBXE0a`) tests that gap without
claiming mesh support. With hole-edge eligibility it emits physical navigation
at 0/90/180 degrees, but those 36 routes fail; the twelve projected 37-degree
routes pass (`local-stair-placements-jeBV3z`). At 37 degrees the physical compiler
reports a hole with no physical area. Neither candidate is published. The
climbing geometry, multi-entrance landing support and rotated contour handling
remain unresolved; enabling physical navigation alone is not a successful fix.

## Three-entrance ladder oak mesh audit (2026-10-06)

The remaining Sherwood ladder oak has a ground entrance and two platform
entrances, all authored as low-type doors. The general seam tool rejects its
first upper contact because no outer landing edge matches. Both upper middle
points lie inside the authored platform hole. This is not evidence that a
larger arbitrary landing extension is appropriate.

Audit `ladder-oak-mesh-audit-Bl0Til` reads only the pinned asset model and
gameplay. Samples along the existing middle-to-outside approaches find platform
mesh within 0.2 height units at 39/41 and 35/41 points. Its 528 near-horizontal
platform triangles form 61 disconnected plank patches. A diagnostic closing
radius of 0.2 game units joins these into one footprint with no hole (0.1 leaves
twelve patches). This is an unpublished authoring candidate, not a changed
compiler connection tolerance. The reconstructed footprint needs review against
the separate tree/collision ownership before replacing the authored hole.

Rung bounding-box centers also disagree with the current climbing plane:
the main rope-ladder centers differ by up to about 81 height units and the
above-platform centers by up to about 186. These are diagnostic center samples,
not measured actor-foot contacts. The current single-plane traversal and the
three approach connections need mesh-based review together. No asset changes
were published from this audit. Its model triangles, footprint candidates,
approach samples and reproduction scripts are retained in the audit directory.

## Published west-treehouse ladder landing (2026-10-06)

Tracing `local-stair-placements-cpAsE5` identifies the unsupported upper receiver.
Candidate `west-treehouse-landing-q9CzTm` adds a matching non-solid/non-opaque
receiver and clears only the upper-platform extension, preserving the existing
hole, collision elsewhere and inside waypoints. All 16/16 moved climbs pass
(`local-stair-placements-RMC1DD`). Full Sherwood (`west-treehouse-level-Cq2WB9`)
retains 10/10 climbs and 2/2 stair routes. Exported upper navigation retains its
hole obstacle; the authoring script explicitly checks the unchanged hole.

`stage-west-treehouse-landing.mjs` produces final stage
`west-treehouse-landing-81Wg1Y`, adding a warning for incomplete ladder mesh and
the upper seam's 3.381-unit sampled discrepancy. All sixteen final-stage routes
pass (`local-stair-placements-UavzCf`). Publication backup
`west-treehouse-publication-20261006` updates the asset and Sherwood pin.
The eight fresh published exports (`local-stair-placements-D5xJXy`) exactly match
the tested final-stage exports. All ten scenes reopen; full published Sherwood
matches its native-tested candidate except for the revised draft warning.
Focused authoring-tool lint/formatting pass. Audit `lift-anchor-support-WuF3DR`
now flags 2/8 ladders, 38/53 stairs and 9/9 walls. Rendered integration and broader
placement coverage remain open; these route checks do not certify visual parity.

## Published central oak ladder seams (2026-10-06)

The narrower candidate `oak-receiver-UyTCZN` clears only the lower-platform
extension: the geometric difference between the corrected and existing floor.
It retains the existing inside waypoints and passes 16/16 moved climbs
(`local-stair-placements-aTP7ak`). Removing the dedicated receiving surface
fails all sixteen routes again (`local-stair-placements-iSbfYD`). Original part
collision remains; the new receiver is non-solid and non-opaque and inherits
the platform material.

`stage-sherwood-oak-landing.mjs` reproduces that candidate from the reviewed seam
stage. Candidate `sherwood-oak-landing-QEUiPo` passes 16/16 climbs at four rotations
and two elevations. Full Sherwood (`sherwood-oak-level-gyWY82`) passes 10/10 climbs
and 2/2 stair routes. Landing review now includes a landing sharing the flight's
own mesh node. The three lower-platform edge checks have 24/41, 14/41 and 38/41
mesh hits, with maximum uncovered distance 1.643 units; the upper-platform edges
each have 41/41 hits. The ladder's incomplete visible mesh remains a limitation.

Final stage `sherwood-oak-landing-MdKXif` adds that explicit visual warning.
Publication backup `sherwood-oak-publication-20261006` updates the asset and
Sherwood scene pin. Fresh published placements (`local-stair-placements-9nrX10`)
pass 16/16 climbs. The local-anchor audit (`lift-anchor-support-bqd3G8`) now flags
3/8 ladders, 38/53 stairs and 9/9 walls. This is gameplay progress, not rendered
parity certification. Focused authoring-tool lint and formatting pass.
Fresh batch `saved-map-exports-pURUNj` reopens and compiles all ten saved scenes,
passes all 84 climb routes and all 71 control apply/reset checks. Published
Sherwood matches the tested full-scene candidate after excluding its revised
draft warning.

The next west-treehouse candidate (`local-stair-seams-X4sFNf`) corrects an outer
platform edge, not the platform hole. Its reviewed candidate bounds allow a
4.2-unit midpoint adjustment and a five-unit landing-edge adjustment. All sixteen
moved routes still fail (`local-stair-placements-cpAsE5`); the upper seam also has
incomplete mesh coverage, with a 3.381-unit maximum sampled discrepancy. This
candidate is unpublished and does not affect the passing saved-scene batch.

## Exact ordinary landing boundaries (2026-10-06)

Ordinary surface landings now retain their pre-grid boundary when connected to
a physical lift, using the same unique-contour and identical-rounded-footprint
checks as raised receiving attachments. Previously only the latter qualified;
the ordinary landing could lose foot support at its rounded seam. Changes below
the clipping-grid error bound do not add redundant precision fields.

With this compiler correction, diagnostic `oak-receiver-RLvu4H` passes all
16/16 climbs and 16/16 unchanged stair routes (`local-stair-placements-D6sqZr`).
Fresh exports after the insignificant-roundoff filter (`local-stair-placements-Ay0leo`)
retain 16/16 passing climbs. Native precise-contour validation passes. The original
seam-only candidate still fails 16/16 climbs (`local-stair-placements-wNFqzp`),
so the compiler correction does not substitute for asset collision/receiver review.
The diagnostic's whole-platform clearance remains unpublished.

A four-rotation compiler regression covers ordinary fractional landing support.
The refreshed changing-climb native fixture differs only by four exact landing
boundaries. All four complete-animation changing-climb tests pass, including
72 mid-climb reopening checks. Editor validation passes 848 tests with two skipped;
app typecheck, focused lint/formatting and the production build pass.

## Sherwood central oak ladder investigation (2026-10-06)

The unpublished seam candidate `local-stair-seams-84DWuZ` corrects the ladder
floor and adjacent platforms, but its eight placements fail all sixteen directed
climbs. A centered-waypoint candidate (`local-stair-seams-JgrTcH`) also fails.
The lower landing cannot bind to the physical door; the upper landing binds.

A dedicated lower-platform receiving volume matching the corrected surface
(`oak-receiver-ZNPvxA`, placements `local-stair-placements-XVaPOv`) retains
part collision but still fails all sixteen routes with the same binding error.
This rules out receiving-volume coverage alone as a sufficient correction.
A diagnostic whole-platform clearance (`oak-receiver-xefXEn`, placements
`local-stair-placements-tE0Piq`) removes collision cuts at the seam and allows
both landings to bind in the traced default placement. All sixteen routes still
fail. Combining that diagnostic with centered waypoints (`oak-receiver-RLvu4H`,
placements `local-stair-placements-9KiS13`) or preserved movement boundaries
(`oak-receiver-ketHUR`, placements `local-stair-placements-gH6GNp`) also fails
all sixteen routes. Actors stall at the lower seam or cannot route down from
the ladder interior. These diagnostics are not published asset corrections:
the clearance needs local ownership/mesh review, and footprint support across
the bound seams remains unresolved. The unsuccessful general waypoint-centering
option was removed from the authoring tool; staged diagnostics remain available.

## Published Lincoln ground contact and precise raised landings (2026-10-06)

The terrain/plateau contact additionally needed precision on the plateau's
blocking contour. Candidate `lincoln-shed-ground-contact-WggrfC` retains both
edges until final clipping. A bounded receiving-edge check accepts clipping-grid
roundoff (2/1048576 units); its regression rejects 0.0001-unit gaps and points
beyond edge endpoints. This permits physical compilation, but the first native
candidate `lincoln-shed-ground-level-aFXW8M` still fails both shed routes.

Raised landings previously recovered their support from an integer boundary
when the receiving volume extended beyond the walking region. Motion areas now
optionally retain `precise_polygon`. Loader validation shares the obstacle
contour validator: the finite valid contour must round to the same grid polygon.
Physical binding unprojects it onto the selected receiver plane and clips to
the real receiver. Missing receivers and live collision remain constraints.
The native regression covers a broad raised receiver and rejects short support.
Replay schema advances to 61 for the added loaded-motion field.

With boundary precision retained, a narrow annex collision still obstructed the
lower approach. Candidate `lincoln-shed-ground-contact-zhA0yC` shifts the three
lower door waypoints three units sideways along the unchanged ladder plane.
The annex collision remains intact. Full Lincoln `lincoln-shed-ground-level-KzPYQi`
passes 8/8 complete-animation climbs, 16/16 stair routes and eleven control checks.
The isolated shed (`lincoln-shed-approach-placement-VY9sId` and
`local-stair-placements-750Y8I`) passes all sixteen moved/elevated climb routes
and eight raised-ground rejections. This does not certify rendered climbing.

Final stage `lincoln-shed-ground-contact-zFZhMF` removes the obsolete projected
contact warning. Publication backup `lincoln-shed-ground-publication-20261006`
updates terrain, keep plateau, shed and Lincoln's pins. All 205 terrain-strip
samples have mesh coverage; the top discrepancy is below 0.0584 units. Fresh
published Lincoln in `saved-map-exports-LeLjCK` matches the tested geometry
after excluding revised warnings. All ten scenes reopen and compile.
The fresh batch passes all 84 climb routes and 71 control apply/reset checks;
the updated game binary builds successfully.
Editor validation passes 847 tests (two skipped), 126 focused compiler/anchor
tests, app typecheck, lint and production build. Native validation passes the
two precise-contour tests, seventeen physical-navigation tests and 29 replay
tests. The pipeline typecheck still reports unrelated `state-delivery.test.ts`
errors; no parity changes depend on those tests.

## Raised receiver eligibility and Lincoln contact investigation (2026-10-06)

Physical ladder eligibility previously checked only authored surfaces, omitting
raised receiving volumes attached to lower navigation. It now also recognizes
point-bound receivers with one unambiguous navigation region, retaining their
physical height and underlying walking boundaries and holes. Segment bindings
still use the existing fallback. A compiler regression checks receiving support
and rejects a hole at the seam. Native complete-animation checks pass both routes
in `receiving-ladder-IOEnhU`. All 123 compiler tests pass; the editor suite passes
846 tests with two skipped. App typecheck, focused lint and production build pass.

Batch `saved-map-exports-Ghc6cc` compiles all ten scenes and passes all 84 native
climb routes and 71 control apply/reset checks. This batch predates the final
precise-boundary eligibility adjustment; the targeted compiler regression also
passes after that adjustment.

Lincoln's saved ladder has an additional ground-boundary gap. The reviewed edge
is owned by both terrain and the keep plateau's ground collision contour.
`stage-lincoln-shed-ground-contact.mjs` stages matching asset-local corrections,
bounded to two units and checked against the inner-bailey plateau mesh.
Candidate `lincoln-shed-ground-contact-iVKQcJ` has 205/205 mesh-covered samples;
the mesh top lies approximately 0.0584 units below the authored receiving plane.
The explicit 0.06-unit mesh review bound does not change runtime tolerances.
Candidate export `lincoln-shed-ground-level-46qvLX` still retains projected
navigation: subsequent clipping leaves a boundary that misses the precise lower
seam. The contact edits are unpublished and require further ownership/clipping
review. No floor or collision was removed to force this check to pass.

## Sloped Lincoln shed ladder landing (2026-10-06)

The seam authoring tool now optionally intersects the lift and landing planes,
preserving inclined roofs. Explicit options bound midpoint adjustments, set a
reviewed placement ground height and extend selected outside approaches within
their existing landing. The placement checker rejects omitted authored lifts
with compiler diagnostics. Mesh review also samples changed outside approaches.
The flat-landing northwest-tower candidates from the old and updated authoring
tools (`local-stair-seams-5MYmXZ` / `local-stair-seams-PnjJ0b`) match within
1e-9. Focused authoring-tool lint and formatting checks pass.

Initial candidate `local-stair-seams-2k6T3r` passes 14/16 complete-animation
routes (`local-stair-placements-Uc4NmR`). At 180 degrees the projected roof
cannot fit the actor footprint at the old outside point; source extraction
relocates the actor and the approach path fails before climbing. Extending the
approach twelve world-XY units onto the same roof fixes the tested placements.
The roof boundary and slope remain unchanged. Its new-drop ground height is
220.001, matching the lower entrance instead of the buried foundation.

Candidate `local-stair-seams-S1xOGI` and placements `local-stair-placements-sRGViM`
pass 16/16 routes at four rotations and two elevations. Eight raised-ground
negative cases reject. Mesh review finds 555/897 ladder floor hits, a maximum
0.886-unit floor-vertex shift, and incomplete visible ladder coverage. All 41
approach samples have mesh hits, but their height discrepancy reaches 0.196
units; none meets the review's stricter 0.1-unit support threshold.

Final stage `local-stair-seams-snSpAF` adds the explicit mesh/contact warning.
Publication backup `lincoln-shed-ladder-publication-20261006` refreshes Lincoln's
pin. Fresh published placements `local-stair-placements-R4oeQe` also pass 16/16
complete-animation routes. All ten saved scenes reopen. Full Lincoln candidate
`lincoln-shed-ladder-level-Siixgq` passes eight climb routes and eleven control
apply/reset checks. Its lower ground contact still does not reach the precise
seam, so this existing placement retains projected navigation with a warning.
That contact and complete rendered integration remain unresolved.

## Published Derby postern ladders (2026-10-06)

Stage `local-stair-seams-0fxVDU` corrects both southwest-postern ladder floors,
door midpoints and local landing edges. The lower entrance remains an external
connection. Geometry matches reviewed candidate `local-stair-seams-bTRQ7j`;
the final stage adds an explicit mesh/rendering warning. Floor vertices shift
at most 1.506 units on the lower ladder and 0.104 on the upper ladder.
Mesh review finds 429/800 and 338/806 floor sample hits respectively; uncovered
samples include exposed feet and heads. Landing discrepancies reach 0.166 units.
This is navigation progress, not complete rendered parity.

`local-stair-placements-vfBoCM` passes 32/32 complete-animation routes at
0/37/90/180 degrees and elevations 0/40. Eight raised-ground negative cases
omit the disconnected lower ladder and preserve the independent upper ladder.
Full Derby `postern-ladder-level-x6hvXh` passes 4/4 climb routes, 28/28 stair
routes and all five control apply/reset checks.

Publication backup is `postern-ladder-publication-20261006`; Derby and Wychford
pins are refreshed. Fresh `local-stair-placements-z2pPhL` descriptors exactly
match all eight tested candidates. All ten saved scenes reopen successfully.
Anchor audit `lift-anchor-support-qC9zlQ` now reports unsupported floor anchors
in 38/53 stairs, 4/8 ladders and 9/9 walls. Fresh baked ZIPs and rendered
character checks remain outstanding.

## Physical ladders and published east-moat correction (2026-10-06)

Compatible planar ladders now use the existing physical floor/collision machinery.
Entry/exit climbing orders retain exact world endpoints; transition animations
keep their positional, posture and sector-change effects. Landing geometry in
ladder assets remains precise until final grid quantization. A physical ladder
requires authored receiving support at both the outside point and midpoint.
This catches the old Derby postern and Lincoln shed height mismatches before
loading; they retain projected navigation with explicit warnings.

Runtime floor checks now bound the error from independently rounded f32 world
coordinates. A captured steep-floor case exceeds the previous 0.001 threshold
by 0.0000014 units; the regression accepts its rounding and rejects real
0.01-unit height gaps. Loader, source authorization and movement share this check.

Authoring candidate `local-stair-seams-Ozmxuf` uses `--climb-seams`,
`--local-landing-edges` and a reviewed `--landing-shift-limit=12`. Floor vertices
move at most 0.337 units; the larger upper-landing adjustment restores the
approach across mesh-supported floor. Landing edge discrepancies reach 0.141
units. The ladder has 351/658 sampled mesh hits, with three uncovered foot
samples visible at 90 degrees; full sprite integration remains unverified.
Intermediate candidates failed due to rounded landing collision; preserving
all surrounding asset surfaces through final rounding resolves those failures.

Final stage `local-stair-seams-nOBhY7` removes the obsolete 180-degree route
warning and retains an explicit ladder mesh/rendering warning. Publication
backup is `east-moat-ladder-publication-20261006`; Leicester and Wychford pins
are refreshed. `local-stair-placements-YmmHHe` passes 16/16 complete-animation
climb routes at 0/37/90/180 degrees and elevations 0/40. Fresh published exports
`local-stair-placements-FpyfRu` match those descriptors exactly. Full Leicester
`east-moat-ladder-level-3lab2Y` passes 22/22 climbs, 16/16 stairs and twelve
control apply/reset checks; fresh full-map geometry matches after excluding
the revised draft warning text.

The expanded placement checks exposed a stair collision regression at 37 degrees:
rounding a clipped landing solid back to f32 turned its edges into crossing
spikes. Landing collision now retains f64 intersection coordinates through
routing and footprint checks, preserving its live obstacle identity. The captured
`precise-landing-clip.json` regression demonstrates that f32 conversion invalidates
the contour and verifies that the retained solid still blocks the actor. All
16/16 published tower stair routes pass alongside the sixteen ladder routes.

Batch `saved-map-exports-FZ3QnW` compiles all ten saved scenes. Native checks
pass 84/84 climb routes, all 71 control apply/reset checks, and Derby's 28/28
stairs. Updated compiler-generated fixtures cover physical ladder controls and
projected walls, including copied isolation, mid-climb barrier closure and 72
reopening checks. Physical routes resume before their blocked-motion abortion;
projected failed requests retain their previous timeout behavior. The focused
movement suite passes 191 tests (twelve ignored); five coordinate tests pass.
All seventeen physical navigation tests pass, including thin-obstacle preservation.
Editor validation passes 845 tests (two skipped), focused lint, formatting,
the app typecheck and production build. The pipeline typecheck is currently
blocked by separate `state-delivery.test.ts` errors. The game build passes.

Anchor audit `lift-anchor-support-duQZVA` finds unsupported floor anchors in
38/53 stairs, 5/8 ladders and 9/9 walls. These counts omit landing compatibility
and are not traversal certification. Fresh baked ZIPs, rendered climbing,
broader rotations and physical wall transitions remain open.

## Climb floor-anchor audit (2026-10-06)

`node refinement/audit-stair-anchor-support.mjs library --all-lifts` extends
the read-only, descriptor-hash-checked audit to ladder and wall definitions.
Report `lift-anchor-support-oWEtgS` covers 70 lifts: 38/53 stairs, 6/8 ladders
and 9/9 climbable walls have unsupported floor anchors. Default invocation
still reports only stairs; its complete result array equals the stair subset
of the expanded report. Focused lint and formatting checks pass.

The east-moat ladder's inside points lie on its planar floor, but its two
middle points miss the boundary by 0.208886 and 0.181409 local units. Their
height differences from the adjoining landings are +0.403543 and -0.411097.
This rules out merely enabling the existing physical stair emitter for this
asset: endpoint seams need review before that emitter can accept it.

The existing 180-degree fixture's `climb-trace-180.log` in
`local-stair-placements-3gX5jX` also reports no actor-sized inside approach at
both ladder doors, followed by failed source extraction on lift layer 5. The
projected corridor cannot fit the actor's movement box. This is distinct from
the local anchor mismatches and remains a runtime/compiler gap. The audit does
not certify climbing routes or change published gameplay; climb animation,
landing transitions and live barriers still need validation with independent
navigation coordinates.

## Published east-moat stair; remaining rotated ladder limitation (2026-10-06)

The remaining upper stair failure came from authoring edge selection. The
default two-unit landing shift limit admitted a short edge near a corner but
excluded the edge across the doorway: its far endpoint needs 2.055524 units.
The tool now requires a selected edge to span the door midpoint, and local
adjustment tracks selected edges explicitly instead of inferring them from
selected vertices. This also avoids treating duplicate/perpendicular neighbouring
edges as seam edges. The previous invocation now rejects with a clear midpoint
error; an explicit reviewed `--landing-shift-limit=2.1` generates the correction.

Candidate `local-stair-seams-bLq2oG` passes **16/16 native stair routes** at
0/37/90/180 degrees and elevations 0/40 (`local-stair-placements-NnbCXs`). Full
Leicester candidate `east-moat-tower-level-0FqnhN` passes **16/16 stair routes**,
**22/22 complete-animation ladder/wall routes** and **12 control apply/reset
checks**. No clearance experiment is retained.

Mesh review finds 309/713 upper-flight samples with its own mesh. All 404 missing
samples have both feet and heads occluded at the four tested rotations. Lower
landing seams have full assembly support; changed upper edges have maximum
uncovered distance 0.386149 units. These sampled checks do not replace rendered
character integration.

The isolated tower's ladder passes **12/16 complete-animation routes**, failing
both directions at 180 degrees at both elevations. Baseline fixture
`local-stair-placements-mu8NKO`, from `east-moat-baseline-mG49fq`, has the same
four failures. The change does not certify this rotation; the published draft
explicitly warns about it. Climb audits use the shared RobinTown animation bank,
not source-level navigation data.

Final stage `local-stair-seams-jkyjcj` differs from the tested candidate only by
two explicit draft issues, authored with `--draft-issue`. Publication backup is
`east-moat-stair-publication-20261006`. Published placement batch
`local-stair-placements-3gX5jX` and fresh Leicester exports match the tested
geometry exactly after excluding those two added warnings. All ten scenes reopen;
Wychford updates an unused reference and has no east-moat tower placement.
Local-anchor audit `stair-anchor-support-MBkpRk` reports **38/53 unsupported stair
definitions**, with Leicester 4/8. Focused authoring-tool lint/format checks pass.
New baked ZIPs, alternate-state visual checks and the rotated ladder remain open.

## Preserve obstacle contours when rounding closes a notch (2026-10-06)

Stair landing obstacles can now recover exact contours even when grid
normalization creates a hole from an open notch. Recovery traces the rounded
obstacle through source-edge rounding cells in the original blocked coverage,
including its interior boundaries. Short adjacent edges can share cells: up to
64 candidate edge combinations are evaluated, clipped to that same coverage and
accepted only if they reproduce the integer obstacle exactly. Their union keeps
all accepted collision; competing source contours, incompatible geometry or a
larger search retain the existing integer fallback. Cleanup uses the established
two-unit clipping-grid tolerance before rounding. Ordinary receiver recovery
keeps its previous unique-edge behavior.

The compiler computes blocked coverage lazily, only for a landing with an
unmatched obstacle. Export still uses `precise_polygon` with the native
same-grid-footprint validation; sector topology and control identities do not
change. `rounded-notch-collision.json` contains two regression geometries.
Tests verify blocked coverage at clipping precision, equal integer footprints,
fractional output and rejection of unrelated or competing source ownership.

The east-moat candidate now emits the recovered lower landing contour. In
`local-stair-placements-RVcJix`, the lower entrance completes its door handoff;
the route subsequently fails toward the upper landing. Complete traversal still
fails **16/16 routes**, so the candidate remains unpublished. The upper landing
requires further collision/asset review. The broader whole-edge authoring
experiment `local-stair-seams-JBILvw` also fails all sixteen placement routes
(`local-stair-placements-XLs7z0`) and is not published.

Validation: **837 editor tests pass, two skipped**; both typechecks, focused lint,
and the production build pass. The native precise-obstacle validator accepts the
zero-degree candidate. Fresh batch `saved-map-exports-ptZMSD` compiles all ten
saved scenes and passes all **71 control apply/reset checks**. Derby retains
**28/28 stair routes**. Derby and published Leicester descriptors are unchanged
apart from the allowed comparison of precise obstacle metadata (their precise
obstacle counts are unchanged too). These are descriptor/runtime checks; new
baked ZIPs and rendered traversal remain outstanding.

## East-moat tower: rounded notch becomes a blocking hole (2026-10-06)

Unpublished candidate `local-stair-seams-4HHZoS` corrects `building-185-lift`
using `stage-local-stair-seams.mjs leicester-east-moat-tower building-185-lift
--local-landing-edges`. Eight placements emit the physical stair, but native
batch `local-stair-placements-3ixHBq` fails **all sixteen directed stair routes**.
Both landings bind; their collision stops actors at entry. The zero-degree
`trace-0.log` and filtered actor report retain the detailed failure. Complete
Leicester candidate `east-moat-tower-level-gKVvDP` passes only 14/16 routes.
The published asset is unchanged.

The lower landing's static obstacle has no `precise_polygon`. Diagnostic
`normalization-diagnostic.log` under the candidate captures a single input
polygon with **one outer ring and no holes**; movement normalization produces
the same region with **an enclosed hole**. Rounding closes an open notch, so the
current exact-hole matcher has no input hole to recover. The rounded blocker
includes the seam edge near [1676,1624] → [1699,1613] in projected placement
coordinates. `region-diagnostic.log` confirms one contributing navigation piece,
with no exact receiving polygon or precise blocker for this hole. This needs a
targeted treatment of collision introduced by rounded topology, while preserving
real obstacles and the integer movement footprint.

Two clearance experiments (`local-stair-seams-Myqjvk` and `local-stair-seams-UcaOIc`)
still failed 16/16 routes, as did an attempted reconstruction of holes formed by
joining multiple navigation pieces (`local-stair-placements-4X2KPl`). Those code
experiments and temporary tracing are removed; none was published. Clearances
are scoped to an asset placement, so changing their node does not isolate one
component's collision.

The stair's mesh review also remains incomplete: 309/713 flight samples hit its
mesh, with the missing samples occluded in the initial view. The lower landing
seam has full sampled assembly support; upper landing edges have discrepancies
up to 0.3862 units. This evidence is insufficient for rendered parity. Local
anchor corrections alone do not justify publishing this candidate.

## Published east-wall turret and terrace contact (2026-10-06)

The east-wall turret now has precise stair endpoints, an aligned local landing
and a matching owner-scoped clearance. Its external approach still requires a
separate receiving asset at the correct height. The terrace owns the corrected
receiving edge; no scene-specific runtime connection or gap tolerance is added.
Both assets remain drafts with explicit rendered-integration warnings.

The first candidate `local-stair-seams-gwGAJ9` passed 32/32 routes in independent
placements (`external-stair-landings-SdFHJr`), but full Leicester candidate
`east-wall-turret-level-bR4nIH` failed two of sixteen routes. Its lower receiving
edge did not reach the physical seam. `trace.log` records the binding failure.
That candidate was not published. The corrected terrace is staged by
`refinement/stage-east-wall-terrace-seam.mjs` from editor placements and pinned
asset definitions.

`stage-local-stair-seams.mjs --local-landing-edges` now limits a landing correction
to the width of the stair and retains the rest of the authored edge. The mesh
reviewer also checks newly inserted edges, including their short connections
back to the old contour. Final turret candidate `local-stair-seams-NruNS0` has
620/620 sampled flight mesh hits and 41/41 upper-seam samples supported by the
assembled mesh. Its connecting edges have 34/41 and 35/41 supported samples;
remaining distances are at most 0.155172 and 0.079997 game units respectively.
The terrace review samples 205 points: 36 have exact support and the remaining
points extend at most 0.267185 units beyond its top mesh. These measured
subpixel authoring discrepancies remain a visual limitation, not runtime slack.

Final combined stage `east-wall-terrace-seam-6HtWTf` is published with backup
`east-wall-terrace-publication-20261006`. Native evidence:

- `external-stair-landings-4eFual`: two independent turret copies at four
  rotations and two elevations pass **32/32 routes**; all **32** missing/raised
  synthetic receiving assets reject. Only external doors receive synthetic
  landings (`--external-only`); the internal landing remains asset-owned.
- `church-terrace-placements-a9QArL`: the actual turret/terrace pair passes
  **16/16 routes** and **16** disconnected cases. Despite the historical fixture
  directory name, these files contain the east-wall turret. Fresh published
  batch `church-terrace-placements-mEknD1` exactly matches all eight descriptors.
  The pair has no controls; the control-only harness rejects an empty audit.
- `east-wall-terrace-level-ZSCwrB`: complete Leicester passes **16/16 routes**
  and all **12 control apply/reset checks**. Fresh published Leicester is exactly
  equal to this descriptor, including warnings.

All ten scenes reopen with verified descriptor pins. Wychford only updates an
unused asset reference; neither changed asset is placed there. Local-anchor
audit `stair-anchor-support-c8ifaH` reports **39/53 unsupported stair definitions**,
with Leicester 5/8 and Derby 0/10. Focused authoring-tool lint and formatting pass.
New baked ZIP and complete rendered actor verification remain outstanding.

## Published church stairs and terrace-owned landing (2026-10-06)

The church-side tower and lower-bailey terrace now publish their reviewed
connection together. The terrace owns its flat top, exact receiving boundary and
owner-scoped clearance, so it carries the landing when moved independently of
the terrain. Its body still obstructs movement below the top. The church retains
an explicit draft warning for incomplete upper stair mesh and unverified
lower-entry character compositing.

`refinement/stage-church-terrace-floor.mjs` stages both definitions from pinned
assets and an editor-compiled candidate; publication uses the normal transactional
asset configuration tool. Candidate `church-terrace-floor-6UJaCJ` is published
with backup `church-terrace-publication-20261006`, refreshing Leicester and Wychford
scene pins. The mesh reviewer samples 205 positions along the adjusted terrace
edge: 41 have exact mesh support and the remaining 164 extend at most
0.232576 game units beyond it. This is an explicit subpixel authoring discrepancy,
not a relaxed runtime connection tolerance.

`refinement/check-church-terrace-placements.mjs` places the two independent assets
on fresh authored terrain at 0/37/90/180 degrees and elevations 0/40. Candidate
batch `church-terrace-placements-FT51Ek` passes **32/32 directed native actor
routes**. All sixteen missing/raised-terrace cases reject the lower connection.
Published batch `church-terrace-placements-cwvmwf` exactly matches the tested
descriptors except for the added visual warning; its eight controls apply/reset
and its sixteen negative cases still reject. Complete Leicester candidate
`church-terrace-level-MJAhPD` passes **16/16 routes** and twelve control checks.

Physical landing collision also removes clipping roundoff confined to an exact
shared stair edge. This applies only to precise obstacles, uses the existing
coordinate-scaled floating-point bound and requires the original solid to extend
into the stair. Tests preserve standalone thin obstacles, real landing-side
walls, unrelated edges and larger strips. Native stair tests pass 16/16;
movement tests pass 191 with twelve ignored. The game build, Rust formatting,
and focused JavaScript lint/format checks pass. The refreshed local-anchor audit
`stair-anchor-support-JWUBgS` reports **40/53 unsupported definitions**, with
Leicester down to 6/8 and Derby still 0/10. These checks do not establish complete
rendered or alternate-state traversal parity.

Fresh published batch `saved-map-exports-XWHhHL` reopens and compiles all ten
saved scenes. Native geometry construction and all **71 control apply/reset
checks** pass; Derby retains **28/28 directed stair routes** with the updated
runtime. Published Leicester geometry matches `church-terrace-level-MJAhPD`
exactly after excluding warning text. This batch contains descriptors, not newly
baked ZIPs; complete image/resource and rendered round trips remain outstanding.

## Church assembly visibility and complete-level gate (2026-10-06)

The seam reviewer now casts projected floor/head rays through initially visible
asset parts, using the editor's placement transform at 0/37/90/180 degrees.
Physical cutout alpha is included through the existing mesh reader. This is
sampled mesh evidence, not a rendered character or alternate-state certificate.
The upper flight has 731 samples without its own floor mesh: at zero degrees,
693 feet and all 731 heads are occluded. The remaining 38 visible feet lie at
heights 95.289–106.409 near the lower entrance. All sampled missing-floor feet
and heads are occluded at the other three rotations. The lower flight has no
uncovered samples. Results are in the candidate's `mesh-review.json` and
`assembled-visibility-review.json` under `local-stair-seams-qCEECD`.

Complete-level batch `church-complete-level-fEhMg4` compares the published
Leicester scene with the candidate. The published scene passes 16/16 directed
routes; the candidate passes only 14/16. Both pass all twelve control apply/reset
checks. The two failures are the lower church flight in opposite directions:
physical sector 98, doors 59/60. Its external landing fails binding because the
receiver does not reach the corrected physical seam. The upper flight passes.
The candidate remains unpublished despite its passing independent-placement tests.

The external receiver belongs to `leicester-lower-bailey-terrace`, part
`building-123`, whose world edge runs approximately [1542,945] → [1613,966]
at height 50.001003. It currently has a projection receiver but no owned walkable
surfaces. Navigation underneath comes from `leicester-terrain`, including
`ground-section-1-0`; the relevant authored edge starts [1543,894] → [1593,909]
in its zero-height navigation frame. The corrected stair midpoint is
[1559.7191,950.03815,50.001003]. Fixing this needs a reviewed external-floor
connection and ownership check, not a global runtime gap tolerance. The filtered
`candidate-trace.log` records the binding warning; its filtered actor report
contains candidate results only, while the initial full audit checked both.

## Preserve exact landing-hole collision contours (2026-10-06)

Compiled motion obstacles can now carry a `precise_polygon` in projected
coordinates. The compiler retains a uniquely matching unrounded hole contour;
the loader requires a finite simple polygon with the same integer-grid footprint.
Grid validation removes exactly collinear vertices and collapsed backtracking
spikes, matching movement quantization. Unrelated and self-intersecting contours
reject. Physical landing binding unprojects the exact contour and clips it to
the supported landing while retaining the same obstacle index and state word.
Ordinary navigation topology and sector numbering are unchanged.
Hole candidates are indexed once per surface group and matched by rounded bounds
before polygon comparison. Optimized placement batch `local-stair-placements-E9TZAB`
is exactly equal to the native-tested descriptors below.

Church placement batch `local-stair-placements-Zcx280`, built from the unpublished
`local-stair-seams-qCEECD` asset candidate, passes **32/32 directed native routes**
at four rotations and two elevations, up from 24/32. All eight control apply/reset
checks pass and eight raised external approaches still reject. This fixes the
rounded-hole obstruction without deleting collision or adding an asset clearance.
The candidate's incomplete mesh coverage remains unresolved; no asset is published
by this change and the library anchor audit remains 42/53 unsupported definitions.

Regression checks cover exact seam contact, real landing collision/state identity,
invalid contours, fractional hole export and collapsed grid spikes. The 126
affected editor tests, both typechecks, 15 stair-navigation tests and 191 movement
tests pass (12 movement tests ignored). Two older projected-navigation tests now
explicitly clear the fixture's physical-stair metadata before checking projected
climb approaches and crossing lines; the level-data suite passes 82 tests with
eight ignored, including the optional exported-descriptor validator. That validator
also passes against the refreshed grid-terrain fixture. Four native fixture files
are refreshed with exact obstacle metadata only; every other descriptor field is
checked unchanged before regeneration.

Fresh saved-scene batch `saved-map-exports-OpI9ci` compiles all ten scenes and
passes native engine construction plus all 71 control apply/reset checks.
Derby also retains 28/28 passing directed stair routes. The full editor suite
passes 835 tests with two skipped after refreshing the four fixtures.
`refinement/check-saved-map-exports.mjs` reproduces this check's descriptor inputs
from saved editor scenes and pinned assets, including terrain and spline walls.
Wychford takes 208 seconds in this run alongside the game build; other maps take
0.15–4.01 seconds. This remains a descriptor check, not a fresh baked ZIP or full
rendered gameplay verification.
The final game build, editor production build, workspace Rust formatting and
focused editor lint/format checks pass.

## Church upper landing collision isolation (2026-10-05)

The unpublished clearance experiment `church-stair-clearance-Omgdxv` adds only
the strip swept by the upper landing seam correction. Its placement batch
`local-stair-placements-noprfL` still passes 24/32 routes: the same eight upper
flight failures remain. The upper landing motion obstacle is unchanged. This
clearance is not a fix and is not published.

`refinement/diagnose-church-stair-hole.mjs` isolates the zero-degree placement
and moves its single upper landing hole obstacle away from the route, retaining
the obstacle count to preserve sector numbering. Diagnostic batch
`church-hole-ablation-3df4R3` passes all four routes, compared with two failures
in the unmodified placement. This deliberately invalid floor is diagnostic only;
it must never be published. Removing the obstacle entirely invalidates sector
references and is not an equivalent diagnostic.

The blocker represents an enclosed navigation hole. Landing binding preserves
the precise outer receiver but clips integer-rounded hole obstacles against it;
the failed upper seam retains a thin obstacle strip. The next correction needs
to preserve and validate precise inner collision boundaries without removing
real holes or changing live obstacle state. The ablation isolates the blocker;
it does not yet prove a safe precision correction or resolve the upper flight's
incomplete visible-mesh coverage.

## Leicester church-side tower seam review (2026-10-05)

Unpublished candidate `local-stair-seams-qCEECD` adjusts both church-side tower
flights. The upper flight's lower receiving edge requires 2.571–4.991 units of
movement, exceeding the default two-unit authoring limit. The staging tool now
accepts an explicit `--landing-shift-limit=5.1` and records it in the review;
the default stays two and no compiler/runtime tolerance changes. This is a
review candidate, not authorization to publish arbitrary gaps.

The lower flight has 790/790 mesh hits, a maximum floor shift of 0.766 units,
and mesh-minus-floor heights 0.006–4.996. The upper flight has only 225/956
stair-part mesh hits, with uncovered distances up to 12.175 units; its visible
mesh and complete traversal still require review. The reviewer now records hits
from every component for each changed landing-edge sample. In this asset,
`building-197` supplies the 95-unit lower landing, with 41/41 assembled mesh
hits even though the receiving surface belongs to `building-192`. Two other
changed landing edges have 31/41 and 41/41 assembled hits; the upper receiving
edge has only 8/41. Missing own-part hits must not be mistaken for missing
assembled geometry, but neither should other-height roof hits count as support.

Placement batch `local-stair-placements-B1EBnR` emits both physical flights at
four rotations and two elevations. Native tests pass 24/32 directed routes;
all eight failures involve the upper flight at 0 or 90 degrees. All eight
raised external-approach cases reject during compilation. The lower-only staging
attempt is `local-stair-seams-EomcNr`; neither candidate is published. Focused
lint/format checks and the mesh-review execution pass. Current published asset
definitions and the 42/53 unsupported-anchor audit are unchanged.

## Publish mesh-derived upper-west stair and terrain contact (2026-10-05)

The upper-west floor candidate `upper-west-mesh-floor-JSFFSf` is now published
with its local platform edge, material plane and clearance. The mesh reviewer
also checks changed flat landing edges against their own asset parts: the upper
platform edge passes 41/41 height samples. The flight's prior mesh review has
751/755 hits and uncovered edge strips no wider than 0.364 game units.

Terrain authoring stage `derby-stair-ground-contact-1CpQD0` corrects the edge in
`ground-section-1-0`: [224,1185] → [224.145420,1184.293687] and [258,1191] →
[257.947882,1191.253142]. All 205 strip samples hit terrain at zero height.
The explicit contact recipe now selects its terrain surface; previous recipes
retain their original default. The combined descriptor
`upper-west-ground-full-bMuSRP` passes native construction, all 28 directed stair
routes and five control apply/reset checks.

Backup `upper-west-mesh-publication-20261005` records publication and verification.
All ten saved scenes reopen with valid pins. Fresh Derby geometry exactly matches
the tested candidate. Eight published placement exports `local-stair-placements-8YjPwf`
exactly match `local-stair-placements-XJDe5H`, which passed 16/16 native routes
after the entry-handoff fix; all eight raised-ground rejections still pass.
Anchor audit `stair-anchor-support-1lmeBH` reports 42/53 unsupported definitions,
with Derby 0/10. This is local-anchor coverage, not certification of every route,
visual state or arbitrary placement. Rendered actor verification remains open.

## Physical stair entry callback preserves floor ownership (2026-10-05)

The upper-west 180-degree fixture exposed a handoff error: after entering the
physical stair sector, the installed order can still be the passage callback
rather than the following physical walk. The post-execution line-crossing guard
looked only at the installed order, so it applied a projected elevation receiver
again. Reconstructing the fractional seam through that plane shifted the actor
off its supported world boundary. This was not an obstacle in the physical floor.

The guard now also recognizes membership in a physical stair sector. Entry
receiver installation retains the authored world seam rather than the rounded
plane evaluation. Script, sound and patch callbacks remain dispatched through
the existing crossing path. A committed descriptor fixture reproduces the whole
actor loop in both directions; it failed before the guard correction and passes
after it. A separate exact-outline geometry query confirms that the supported
seam itself is traversable. Stalled-route diagnostics now include world position
and queued orders to distinguish receiver drift from route/collision failures.

With the fix, `local-stair-placements-XJDe5H` passes all 16/16 directed routes
across four rotations and two elevations. Published Derby
`lower-west-contacts-full-wgsNlT` retains 28/28 passing routes. The full Derby
mesh-floor candidate `upper-west-mesh-full-cqrjt3` remains 26/28: this separate
asset/terrain contact problem is not fixed by the runtime change. The candidate
remains unpublished. All 15 stair-navigation tests, 191 movement tests and 33
door-passage tests pass (12 environment-dependent movement tests ignored).
The game build and workspace formatting check pass.

## Mesh-derived upper-west floor candidate (2026-10-05)

`refinement/stage-upper-west-mesh-floor.mjs` derives a smooth ramp from the pinned
flight's lower edge and upper tread center, retaining the union of tread outlines
rather than the wider underside. Duplicate decoded tread vertices are joined on
a 0.001-unit authoring grid. The end corners are seated on the approach heights;
the asset-local platform seam, door approaches, material plane/footprint and
movement clearance are updated together. No runtime or compiler tolerance changes.

Candidate `upper-west-mesh-floor-JSFFSf` emits physical navigation in all eight
placement exports (`local-stair-placements-XJDe5H`). Native actor tests pass
14/16 directed routes; both failures are lower entry at 180 degrees, with a
rounded lower-boundary blocker while the actor is already in the stair sector.
All eight raised-ground rejection cases pass during fixture compilation.
The full saved Derby candidate `upper-west-mesh-full-cqrjt3` passes 26/28 routes.
This candidate is unpublished and does not replace the passing published Derby.

Mesh review has 751/755 sampled hits, maximum uncovered edge distance 0.364,
and mesh-minus-ramp heights -2.405 to +8.208 units. The boundary moves up to
12.503 units from the previous definition, reflecting the full visible flight.
The review now reports nearest-boundary vertex distance for topology changes;
index-corresponding vertex shifts are reported only for equal vertex counts.
The earlier underside-quad candidate (`upper-west-mesh-floor-FAaool`) also passed
14/16 routes but included side strips with large height disagreement. It was
superseded by the tread-outline candidate. Focused lint and formatting pass.
Remaining gates include exact landing/ground contact, the 180-degree entry
blocker, complete placed traversal, and visual review of the changed platform.

## Upper-west stair profile review (2026-10-05)

The mesh review now samples each horizontal triangle's center, records its
navigation height and containment, and checks coverage by the flight itself and
other asset parts. This distinguishes underside faces and covered steps from
exposed treads without treating a stair-only mesh sample as the assembled asset.

Rechecking `local-stair-seams-RZsuxX` finds 26 exposed tread triangles, 22 with
centers inside navigation. Navigation-minus-tread height ranges from -5.216 to
+14.840 units; the upper two treads are outside the navigation floor. No adjacent
part covers these centers. The model's top tread extends beyond the authored
upper seam, confirming a mesh/floor disagreement rather than only a snapped
door or a hidden extension beneath the platform. This candidate remains
unpublished pending coordinated flight/landing geometry correction.

For comparison, the published lower-west stair review
`published-lower-west-profile-PZ1LEj` finds all 38 exposed horizontal triangle
centers inside navigation, with navigation-minus-tread heights from -5.534 to
+0.000708 units. That is consistent with a smooth ramp beneath stepped treads.
Both reviews execute successfully; focused lint and formatting checks pass.
These profile samples supplement, rather than replace, full mesh coverage,
rendered actor checks and native traversal tests.

## Publish lower-west stair and external contacts (2026-10-05)

Published `derby-lower-west-access-stair` together with its terrain-owned lower
contact and `derby-lower-west-curtain` upper receiving edge. The compiler uses
only these asset-local definitions; the authoring scripts use the saved Derby
placement to convert the reviewed contact into each owner's local frame.

The terrain review `derby-stair-ground-contact-lVSHEz` has 205/205 mesh-supported
samples and edge shifts of 0.687 and 0.085 game units. Upper-wall review
`lower-west-stair-landing-zoNKot` shifts the authored edge by 1.710–1.826 units.
167/205 samples hit the wall's top mesh; the other 38 are 0.106–0.229 units
outside it. A joint stair/wall mesh overlay (`lower-west-stair-landing-lDakko/contact.png`)
shows the narrow mesh seam discrepancy. Publication explicitly uses
`--reviewed-mesh-edge=0.25`; the default authoring check still rejects unsupported
samples. No export/runtime attachment tolerance is widened. The report retains
`supported: false`, the separate review acceptance, mesh hashes and sample hits.

Combined descriptor `lower-west-contacts-full-wgsNlT` passes 28/28 directed native
stair routes, all five control apply/reset checks and native geometry construction.
Publication backup `lower-west-contact-publication-20261005` includes the edits
and scene pins. All ten saved scenes reopen and the published Derby descriptor
exactly matches the tested candidate. Published external-landing fixture
`external-stair-landings-aOM0wA` passes 32/32 native routes for two copies at four
rotations and two heights, plus 64 missing/raised-landing rejections. Focused lint
and formatting checks pass. The refreshed anchor audit
`stair-anchor-support-uNVYcg` reports 43/53 unsupported definitions, Derby 1/10.
The remaining Derby upper-west stair still needs mesh/floor review. These checks
do not establish rendered parity or every possible placement/state combination.

## Recover precise receivers after movement-grid splitting (2026-10-05)

`restoreReceivingBoundary` recovers a normalized movement piece from its
pre-grid source contour. Each emitted edge must match a unique source edge
through both endpoint rounding cells. Shared corners retain their source
vertices; new split corners connect source-edge points within their common
rounding cell. The candidate is clipped to the source outer contour and must
round to exactly the emitted region. Holes retain their separately compiled
movement blockers. Ambiguous, disconnected or nonmatching recovery falls back
to the existing rounded receiver; no original map data is consulted.

Two regressions cover the 90-degree split and a 180-degree half-grid endpoint.
They assert no coverage outside the source, exact rounded-region identity,
retained source vertices, and rejection of unrelated or competing contours.
The first implementation fixed only the 90-degree case (`external-stair-landings-U7XDvk`,
24/32 routes); handling the half-open rounding cell fixes the remaining eight.
Final ordinary-landings export `external-stair-landings-vGvzYT` passes 32/32
directed native actor routes and all 64 missing/raised-landing rejections.
The fixture tool now stores its synthetic descriptors and input scenes so these
compiles can be reproduced without reconstructing the authoring inputs.

The 195 affected editor compiler/export/quantization tests pass, both TypeScript
checks pass, focused lint/format checks pass, and the production editor build
passes. Static ten-map snapshots `precise-split-all-maps-sNwsCq` pass native
construction and all 71 compiled control apply/reset checks; Derby passes all
28 stair routes. Those snapshots omit splines and editor state assignments,
so they are not full saved-scene or rendered-ZIP parity evidence. The lower-west
stair candidate remains unpublished pending its two real Derby landing contacts.
The separate batch `precise-split-saved-scenes-PwH9lV` compiles the actual saved
scenes with splines and editor state assignments intact. All ten descriptors
construct native geometry and all 71 controls apply/reset. Wychford includes
17,478 sight obstacles and 28,825 elevation boundaries in this batch. These
remain best-effort descriptors with warnings, not new baked ZIPs or complete
rendered/gameplay certification. Recovery is only attempted for possible stair
landings and stops at the first ambiguous edge. Optimized copied placements
`external-stair-landings-EvVoQF` exactly match all eight native-tested descriptors.
All ten optimized saved-scene descriptors also match their native-tested JSON
(`optimized-equality.json`). Wychford compilation took about 98 seconds; the
other maps took 0.1–3.5 seconds. This check does not establish fast export for
large spline-heavy maps. Comparison uses serialized JSON: signed zero in some
in-memory coordinates serializes as zero and is not an exported difference.
The refreshed lower-west candidate `lower-west-split-fixed-full-4KdsaN` still
fails its two full-Derby routes (26/28 overall). The compiler fix therefore does
not substitute for reviewing and correcting its actual neighbouring asset seams.

## Lower-west stair: independent landing fixtures and open precision gap (2026-10-05)

Candidate `local-stair-seams-onfJOY` corrects the two external endpoints of
`derby-lower-west-access-stair`. The maximum floor shift is 0.256274 units;
723/725 sampled points hit the mesh, with uncovered edge distance at most 0.123.
The tread spacing is about 8.33 units. No landing geometry is added to the stair
asset, and neither endpoint borrows a source-level receiver during export.

`refinement/check-external-stair-landings.mjs` authors two separate synthetic
landing assets and places two independent assemblies at each of four rotations
and two elevations. It accounts for the groups' different rotation pivots.
Landing pads extend 20 units beyond each side of the stair and 80 units outward.
These static gameplay fixtures have no real model resources and are not editor
library publications or rendered tests. Removing or raising either landing of
either copy omits only that copy's stair; all 64 negative cases pass.

With `--preserve-landings`, exports `external-stair-landings-mkbpMW` pass all
32 directed native actor routes. Ordinary landings in
`external-stair-landings-cSu35w` fail 16/32 routes at 90/180 degrees. Collision
and integer-grid normalization split the upper landing into several motion
regions. The compiler only preserves an exact receiving contour when one raw
contour rounds to exactly one emitted region; these split pieces fall back to
rounded receivers. Runtime correctly reports that the upper receiver no longer
reaches the physical seam. Preserved boundaries demonstrate the correction
needed but are not a general fix for ordinary split landings. An earlier
stair-width-only pad fixture (`external-stair-landings-BRzqdR`) also failed eight
approach routes; it is not a passing narrow-landing guarantee.

Full Derby candidate `lower-west-seam-full-7vCsxi` fails two of 28 routes at this
stair. The lower contact belongs to terrain edge `[407,1768]`–`[442,1760]`; the
upper receiver belongs to `derby-lower-west-curtain/building-045-walk-0`.
Projected candidate endpoint shifts for those neighbouring edges are under two
units, but their mesh support has not yet been reviewed or published. The stair
candidate remains unpublished until both placed connections and ordinary split
landing export are resolved. The library anchor count remains 44/53 unsupported.

## Gatehouse seams and partial landing receiver binding (2026-10-05)

The gatehouse floor omitted part of its upper tread. Baseline mesh review is
`gatehouse-floor-review-GvRNU4`; candidate `local-stair-seams-z0PFF8` uses an
explicit `--floor-shift-limit=3` authoring override (default remains two units).
The maximum correction is 2.566319 units. Mesh review finds regular 7.08-unit
treads and 1,293/1,297 sample hits; the four uncovered samples are at most 0.180
units from the mesh edge. Both endpoint landing boundaries are corrected with
the floor, and the selected authoring limit is recorded in the review.

Initial placement exports `local-stair-placements-ZJiVPP` failed four of sixteen
routes at 180 degrees, and full Derby `gatehouse-seam-full-VuUQfO` failed two of
28. Runtime tracing showed the upper landing could not bind: its motion region
also contains a receiver at another height. Requiring the selected receiver to
equal the entire region clipped away the exact seam. Binding now accepts the
receiver when its rounded footprint is contained in that region. A regression
covers partial coverage and rejects a receiver extending outside the region;
existing wrong-height, short-receiver and real-gap rejection tests still pass.

With this fix, all 16 placement routes and eight control apply/reset checks pass,
as do full Derby's 28 routes, native construction and five controls. Fourteen
stair-navigation tests and 190 movement tests pass (12 movement tests ignored).
The gatehouse definition is published with backups in
`gatehouse-seam-publication-20261005`; Derby's pin is refreshed. All ten scenes
reopen, published Derby exactly matches the tested candidate, and all eight fresh
placement exports in `local-stair-placements-mU2vt1` exactly match the tested
descriptors. The local audit `stair-anchor-support-pTbPPE` now reports 44/53
unsupported definitions, including two remaining Derby stairs. Complete rendered
traversal and broader state/placement verification remain open.

## East-bailey stair and terrain contact publication (2026-10-05)

The terrain contact staging tool now also handles the explicitly reviewed
east-bailey contact. Candidate `derby-stair-ground-contact-VvZ10Z` projects the
terrain edge `[1581,1609]`–`[1629,1619]` onto the corrected stair's zero-height
seam. Endpoint shifts are 1.164712 and 0.015989 units. All 205 sampled points
between the old/new boundaries intersect the pinned terrain mesh at height zero.
The stair candidate `local-stair-seams-sXM21T` has regular 7.5-unit treads,
774/783 sampled mesh hits, uncovered edge strips at most 0.374 units wide and a
maximum floor-boundary shift of 0.435 units. Its smooth walking ramp follows the
reviewed tread footprint; this is not a rendered actor-compositing certification.

Combined full Derby `east-bailey-ground-full-sXZx9L` passes native construction,
28/28 directed stair routes and all five control apply/reset checks. Publication
backs up both asset definitions and the refreshed Derby scene pins in
`east-bailey-ground-publication-20261005`. All ten saved scenes reopen, and a
fresh published Derby compile exactly equals the native-tested candidate.
Published placements `local-stair-placements-M0PdP7` exactly match all eight
previously tested descriptors (16/16 routes); eight 20-unit raised entrances
correctly omit traversal. The refreshed audit `stair-anchor-support-9Qme5Q`
finds 45/53 unsupported local stair definitions, including three remaining Derby
stairs. No export-time source data or gap repair was introduced.

## Physical stair exit receiver handoff (2026-10-05)

The split-landing actor regression failed before the fix: the exit retained the
outside waypoint's receiver while placing the actor at a seam in another strip.
Physical exits now query the actual seam. A missing seam receiver is retried at
most four f32 steps toward the outside, covering independently rounded boundary
vertices without reaching the distant waypoint. The east-bailey 90-degree seam
miss was approximately 0.00000048 units, below its 0.00012207-unit f32 step.

The strengthened regression checks receiver identity during every landing tick
for both doors and directions. All 223 movement/door tests pass (12 ignored).
`local-stair-placements-WqkkUh` now passes 16/16 directed actor routes across
0/37/90/180 degrees and elevations 0/40, including the previously failing
180-degree exits. The intermediate exact-seam-only change failed two 90-degree
exits; the bounded boundary probe resolves those too. These checks use the native
actor loop, not complete rendered sprites. The east-bailey definition remains
unpublished because its separate full-Derby ground-contact gap remains open.
The published-geometry Derby fixture `derby-lower-east-full-srfDW5` still passes
all 28 directed stair routes after the runtime change.

## Derby curtain stair batch and lower-east publication (2026-10-05)

Three explicitly staged local corrections use the stair's external ground
endpoint and its asset-owned upper landing. All 24 placements emit physical
navigation and reject their raised, disconnected ground entrances. The placement
checker now accepts an absent lift array when every traversal was omitted.
`work/map-compile/derby-wall-stair-batch-08ar745d` passes 46/48 directed actor
routes: lower-east and upper-west pass 16/16 each; east-bailey passes 14/16.

Lower-east candidate `local-stair-seams-4ZxBUm` moves its floor by at most 0.877
units. All 1,190 mesh samples intersect the stair; tread residuals span about
-7.11 to 7.10 units around its unchanged smooth ramp. The separately compiled
full Derby `derby-lower-east-full-srfDW5` passes 28/28 stair routes and all five
control apply/reset checks. The asset and Derby scene pin are published with
backups in `lower-east-stair-publication-20261005`. All ten scenes reopen and
published full Derby exactly equals the native-tested candidate. Published
placement exports are in `local-stair-placements-nyGMS3`. The refreshed local
anchor audit (`stair-anchor-support-lHwZxH`) has 46/53 unsupported definitions,
including four remaining Derby stairs.

East-bailey candidate `local-stair-seams-sXM21T` is unpublished. At 180 degrees
and both heights its downward exit leaves the actor on terrain receiver 18 while
the actual position queries receiver 17; these are adjacent flat terrain triangles
with identical material. This is away from their shared boundary, so the test
correctly rejects the mismatch. Door handoff currently selects a receiver at the
outside waypoint before restoring the physical seam position; this is a concrete
lead for a focused regression, not a verified fix. The combined east-side full
Derby candidate (`derby-wall-stair-full-kUdq1z`) also fails both east-bailey routes
at its ground contact. Both issues remain open.

Upper-west candidate `local-stair-seams-RZsuxX` is also unpublished despite its
passing routes. Only 805/822 samples intersect its selected mesh, uncovered samples
are up to 1.842 units from mesh edges, and mesh/floor height residuals reach -80.12.
The visible stair extends beyond the authored upper boundary. A small seam snap
does not establish geometry parity for that asset; model/frame review is required.

## Published ground contact and bounded landing queries (2026-10-05)

`refinement/stage-derby-stair-ground-contact.mjs` reviews the two terrain edge
endpoints against a pinned terrain model and compiled candidate frame. The terrain
model uses a bare Z-up root; standalone building models have a map wrapper and
require a different frame conversion. In the correct frame, all 205 samples across
the added contact strip intersect mesh at exactly zero height. The two terrain
endpoints move 0.6632 and 2.4224 units; no collision holes or other edges change.
Combined staged edits and mesh evidence are in
`work/map-compile/derby-stair-ground-contact-wcawND`.

The first combined export (`east-hall-ground-full-HzQaZL`) binds the ground but
still fails two routes and takes 309.81 seconds. A stair query processed the whole
ground support and its distant obstacles. Routing now clips query geometry to
the stair bounds expanded by the actor's full half-size. Actor centers remain
on the stair, and the effective footprint is one unit smaller, so distant geometry
cannot affect support and the new clipping boundary cannot restrict valid centers.
A regression compares local and huge landings with 100 distant obstacles, retaining
the same detour while a nearby barrier still blocks traversal. Thirteen stair
tests and 190 movement tests pass (twelve movement tests ignored). The same
two-failure diagnostic takes 11.87 seconds after this change, with tracing enabled.

The remaining failure was in preserved-boundary export: ordinary generated areas
retained their pre-grid receiving contour, but preserved terrain areas did not.
They now retain that contour too. A zero-height fractional landing regression
fails without the change and passes with it. Integer movement geometry and live
collision remain authoritative; exact receiving geometry is used only under the
existing runtime identity checks.

`east-hall-ground-precise-aGkMlb` passes **28/28 full Derby stair routes** in
8.01 seconds and all five control apply/reset checks. The lower stair and terrain
definitions are published with backups in `east-hall-ground-publication-20261005`.
All ten saved scenes reopen, and the published full Derby descriptor exactly
matches this native-tested candidate. The published hall placement fixture is
`local-stair-placements-Pj4yie`; its fresh native audit passes **64/64 routes**
in 29.84 seconds, and all eight raised-ground rejection checks pass.
The compiler/export/spline suites pass 198 tests, both TypeScript typechecks and
focused lint pass, and the Rust game build and formatting checks pass. These
results do not certify complete sprites, rendered traversal or remaining assets.

## Lower east-hall stair and external ground contact (2026-10-05)

The seam staging recipe now accepts explicitly named `--external=` door IDs.
These require no local landing owner, retain their authored outside position and
create no substitute receiving floor. Unused or conflicting selections reject.
The compiler still resolves the actual placed receiver. The placement checker
raises these assets by 20 units and asserts that disconnected traversal is omitted.

Unpublished candidate `work/map-compile/local-stair-seams-8uUfqO` corrects the
east hall's lower stair and upper landing, retaining the published upper stair.
Floor movement is at most 1.349 units. Mesh sampling intersects 639/641 points;
the two remaining samples lie within 0.205 units of the mesh edge. The irregular
treads remain above and below the unchanged smooth navigation plane.
`local-stair-placements-DHIW64` passes **64/64 directed actor routes**, covering
both stairs at four rotations and two elevations. The repeated compiler fixture
`local-stair-placements-yKi1yH` additionally rejects all eight raised ground
entrances while preserving the independent upper stair.

Full Derby in `east-hall-lower-full-v1UV2F` passes control apply/reset but fails
two of 28 stair routes. Runtime tracing isolates the lower ground receiver:
the terrain asset's `ground-section-0-0` boundary edge from [1375,1381] to
[1411,1365] remains about 1.518 units away from the corrected midpoint
[1391.848643,1371.850417]. This is a real authored boundary gap, not f32 noise.
Projecting those edge endpoints onto the zero-height stair seam would move them
0.664 and 2.423 units respectively; that terrain change requires geometry review.
No asset definitions or scene pins were published from this candidate. The
published full Derby remains the previous passing version.

## Published east-hall upper stair seams (2026-10-05)

`refinement/stage-local-stair-seams.mjs` authors explicitly selected local stair
corrections with descriptor pins, bounded floor/landing movements and unique
local landing ownership. It retains intermediate side-boundary heights and door
permissions. External/ambiguous landings require separate review; a gatehouse
attempt also rejects its still-unsupported upper midpoint rather than publishing.

The first east-hall upper stair candidate compiled physical navigation but failed
all six upper-stair routes per placement. A wider landing had only one endpoint
adjusted because its other endpoint lay beyond the stair span; runtime tracing
confirmed that neither lower entrance could bind landing support. The revised
recipe adjusts both endpoints of the complete matching near-coplanar landing edge.

Candidate `work/map-compile/local-stair-seams-GWqLz7` changes the upper floor by at
most 0.323 units and its landings by at most 0.532/0.770 units. All 807 sampled
floor points intersect the pinned stair mesh; the smooth ramp lies within the
approximately 6.5-unit tread steps. Permissions, control definitions and the
lower stair are unchanged. The candidate includes an asset-owned floor clearance.

`refinement/check-local-stair-seams.mjs` inserts the complete asset independently
over authored terrain. `local-stair-placements-u5SepC` tests rotations 0/37/90/180
and elevations 0/40: all **48 upper-stair directed routes pass**. The complete
audit is **60/64**, with four failures on the unchanged lower stair at 180 degrees.
These are initial-state actor-loop checks without complete sprites. Full Derby
in `east-hall-seam-full-ljO2YV` passes **28/28 stair routes** and all five control
apply/reset checks; its upper stair no longer falls back to projected navigation.

The asset and Derby scene pin are published, with backups under
`work/map-compile/east-hall-seam-publication-20261005`. All ten scenes reopen.
The eight `--published` exports in `local-stair-placements-K2NXTu` and the full
published Derby descriptor exactly equal their native-tested candidates.
The refreshed `stair-anchor-support-pAMh3U` audit has 48/53 definitions with local
anchor-support issues (Derby 6/10). Rendered traversal, other stairs and full-map
parity remain unfinished.

## Library stair-anchor support audit (2026-10-05)

`refinement/audit-stair-anchor-support.mjs` reads hash-verified library descriptors
without source maps or inferred ground. It records each stair's planar residual,
inside/middle anchor containment and height error, distance to the floor edge,
middle-to-landing height difference and possible flat local landing owners.
Results are in `work/map-compile/stair-anchor-support-SdVBJ4/report.json`.
All 53 authored stair floors are planar, but 49 have at least one unsupported
anchor: Derby 7/10, Leicester 8/8, Lincoln 6/6, Nottingham 11/12, Sherwood 1/1
and York 16/16. Derby's unsupported middles are 0.033–0.954 units from their
floor boundary and lie on the extrapolated plane to numerical precision.

This is an asset-local audit, not a count of failed exported routes. It does not
resolve neighbouring assets, deform splines or test actor footprints and control
states. Missing local landing candidates can legitimately be terrain or separate
assets. Corrections must preserve those external connections and be checked after
placement; no gameplay definitions are changed by the audit.

## Published keep component seam corrections (2026-10-05)

The composite candidate's eight floor edits, seven door-midpoint edits and three
full-stair clearances now belong to the reusable central gallery and west tower.
`refinement/stage-keep-component-seams.mjs` checks source descriptor pins, local
coordinate frames and baseline surfaces before transferring only reviewed fields.
Permissions, room membership, appearance and control definitions are retained.
Review artifacts are in `work/map-compile/keep-component-seams-Dccpop`.

`refinement/check-keep-component-seams.mjs` independently inserts all four keep
components over authored terrain at rotations 0/37/90/180 and elevations 0/40.
Placement accounts for each component's rotation pivot and camera foreshortening;
transformed part coordinates are checked against the composite assembly.
`work/map-compile/keep-component-placements-ow6STQ` passes **80/80 directed native
actor routes** and all eight control apply/reset checks. This is initial-state
walking without complete sprites, not rendered traversal certification.

`refinement/review-keep-stair-seams.mjs` checks pinned models and overlays the
published/candidate floors with the mesh. Vertical sampling finds stepped treads
about 6.4–6.5 units apart on stairs 156 and 134, explaining why few complete mesh
triangles lie within two units of the smooth navigation ramp. Upper-mesh minus
floor residuals range from -0.05 to 6.41 and -0.16 to 6.34 respectively; stair 154
has residuals 0.004–0.129. Across the three floors, 763/772, 746/748 and 872/888
samples intersect the mesh. Uncovered samples lie within 0.238, 0.015 and 0.066
units of mesh edges. This sampled geometry review does not replace actor rendering.

Publication through `configureAssetGameplay` updates both descriptors and the
Derby/Wychford scene pins, with backups in
`work/map-compile/keep-component-seams-publication-20261005`.
The verification script's `--published` mode reads installed gameplay rather than
overriding it; all eight descriptors in `keep-component-placements-RcPYq9` exactly
match the native-tested staged exports. The composite asset itself is not published.
All ten saved scenes reopen with valid descriptor pins. A fresh full Derby export
constructs twelve lifts and passes native apply/reset for all five controls.
Seven other stairs still warn that authored endpoints lack physical floor support
and retain projected navigation: buildings 078, 081, 196, 012, 038, 265 and 114.
The keep corrections do not resolve those independent asset seams.

An independent light-probe regression also exposed rounding rejection of a valid
authored contour anchor. Layer selection now accepts coverage by either the raw
authored contour or its emitted integer contour; serialized geometry is unchanged.
The outside-both regression still rejects, and deformed spline probes retain their
existing behavior. All 197 affected compiler/export/spline tests and focused lint
pass. Both TypeScript typechecks, formatting and whitespace checks also pass.

## Rounded landing seams and keep clearance diagnostics (2026-10-05)

Material clipping inserts vertices along straight receiving edges. Rounding those
vertices individually introduces kinks absent from the emitted motion boundary.
Runtime receiver-identity checks now remove collinear noise within the encoded
coordinate precision before rounding, then remove integer-grid duplicates/spikes.
All seven landing receivers in the unrotated staged keep match after this cleanup.
A regression retains a real bend while removing a redundant fractional vertex.

Landing binding now checks shared-edge overlap and door seating within the same
f32 coordinate-error budget used by physical route endpoints. It still requires
nonzero shared length and matching floor heights. A wider landing rotated through
37/90/180/270 degrees binds and supports stair entry; a 0.01-unit gap rejects.
Foot support closes representational cracks with a bounded outward/inward bevel
operation before actor-footprint erosion. Live obstacles remain authoritative.
Straight supported swept footprints bypass construction of the full actor-center
region; other routes retain full geometry for visibility while dropping redundant
candidate vertices within the coordinate-error budget.

The intermediate binding-only audit in
`work/map-compile/keep-tolerant-bindings-1b2bi0d1` finishes with **4/80 routes
passing** and no landing-binding warnings. This supersedes the previous failure
to bind every keep landing, not complete keep traversal. Rounded-buffer experiments
were stopped after exposing excessive pathfinding work; the retained correction
uses bevels and avoids redundant route candidates.

The staged seam correction had also left some stair clearances on their old
floor boundaries, producing thin collision strips across the corrected floor.
`work/map-compile/keep-placements-xzcrsx` is a separate diagnostic candidate with
asset-owned clearances covering the corrected stair floors. The final audit is
retained separately in `work/map-compile/keep-final-clearances-g2q5u6qs`: **80/80
directed actor routes pass**, covering three stairs at rotations 0/37/90/180 and
elevations 0/40. The report marks `audit_finished` and `complete` true. Its scope
is initial-state walks between every entrance pair, with `complete_sprite` false;
this is not complete-animation or rendered actor validation. The audit took
108.56 seconds while the game was building.

All eight candidates also pass native geometry construction and control apply/reset
(one control each). The candidate must receive geometry/ownership review, and its
corrections must migrate into reusable component assets before publication. No
asset publication or full-map parity claim follows from these diagnostics.

Validation of the final runtime changes: twelve stair-navigation tests and 190
movement tests pass (twelve movement tests ignored); the movement suite takes
14.95 seconds. The 32 ordinary level-loading tests pass (two ignored). Formatting,
whitespace checks and the game build pass. The fast swept-footprint path requires
positive clearance: an existing regression rejects a zero-width actor-center
corridor and falls back to the full solver for boundary cases.

## Precise landing receivers and unresolved keep binding (2026-10-05)

The compiler retains pre-grid receiving contours when a clipped source region
rounds to the same emitted navigation boundary. Physical stair landings use this
contour, including zero-height landings when fractional geometry needs preserving.
Integer-aligned zero-height landings keep their existing implicit receiver. Rings
are normalized without repeating their closing vertex.

Runtime landing binding combines active coplanar receivers owned by the same
motion sector/layer. It accepts their precise boundary only if rounding reproduces
the motion boundary; otherwise it retains the motion/receiver intersection. Live
collision continues to use the actual motion area's obstacle state. A native
regression accepts a quarter-unit seam lost by integer rounding while rejecting
short and unrelated oversized receivers. An editor regression verifies the
zero-height fractional receiver and its physical door endpoint.

The staged eight-placement batch `work/map-compile/keep-placements-us6eTG` still
fails **all 80 directed actor routes**. A filtered diagnostic in
`work/map-compile/keep-precision-binding-y5lnsdoc` rejects all seven landing doors
in the unrotated, zero-elevation placement as not reaching their physical doors.
Exported receiver-to-midpoint distances there are below 0.0000004 world units;
the remaining runtime seam rejection needs investigation. No keep definition is
published by this change, and these results do not establish traversal parity.
A fresh batch after ring normalization, `work/map-compile/keep-placements-sq7ORI`,
also fails all 80 routes. In its unrotated placement, six of seven door receivers
have rounded symmetric-difference areas of 4.5–33 square units against their
owning motion boundary; one matches exactly. These are final emitted receiving
contours, so pre-material contour matching alone is insufficient. Both final
receiver identity and numerical seam binding remain open.

Validation: 191 focused editor/compiler tests, both TypeScript typechecks, focused
lint, eleven native stair-navigation tests and 190 movement tests pass (twelve
movement tests ignored). The game build passes. The existing native descriptor
fixtures remain unchanged.

## Staged keep seams expose landing precision loss (2026-10-05)

`work/map-compile/keep-door-seams-WR2Oyl` stages asset-local seam corrections from
the unchanged keep candidate. Each stair keeps its existing plane. Its end
vertices move along the slope to the adjacent asset-owned landing heights; door
midpoints are recomputed on their outside-to-inside segments. Nearby landing
vertices align with that seam. The matching landings are selected by local
geometry and height, without scene or level-data lookups. Maximum stair-floor XY
changes are 0.334602, 0.313056 and 1.104837 units for 156, 154 and 134 respectively.
The largest midpoint XYZ change is 2.096771 units. These changes are staged for
review, not published.

The candidate exposed two numerical issues now fixed: tangent solid clipping
could produce negligible fragments that aborted physical compilation, and f32
encoding could place an exact boundary anchor just outside its independently
rounded edge. Physical collision area calculation now uses a local origin and
discards generated fragments below the existing physical area threshold before
allocating control pairs. Native boundary validation accepts only the same
coordinate-rounding budget used by physical route queries; tests still reject
meaningful unsupported positions.

`work/map-compile/keep-placements-kPG2tq` emits all three physical stairs at each
of eight placements and loads natively. **All 80 directed actor routes fail** at
entry. This is not a publishable candidate. The isolated trace in
`work/map-compile/keep-bindings-zw__462v` finds no bound landings: five doors fail
receiver reach and two fail shared-edge matching in the zero-rotation case.
Their exact midpoints are 0.121979–0.471218 units from the integer-rounded landing
navigation edges. Some receiver contours also differ. Preserve actual authored
landing support through compilation and binding before retrying publication;
relaxing actor-sized support checks would hide the missing geometry.

The audit's `ROBIN_LIFT_TRACE` mode now enables loader warnings. Focused compiler
suites pass 141 tests; all 63 map-export tests pass. Native movement passes 205
tests (12 ignored), and the new level-data boundary validation test passes.
Both editor typechecks, focused lint, formatting and the game build pass. The unchanged keep baseline
remains at 56/80 successful routes; this staged candidate does not supersede it.

## Refreshed keep placement audit and unsupported door seams (2026-10-05)

`work/map-compile/keep-placements-9UUrtV` recompiles the precision-preserving
candidate from `keep-placements-RegQxL` with the current ordinary export pipeline.
It retains three stairs, six building groups and one control at each of eight
placements (rotations 0/37/90/180, elevations 0/40). The native directed actor
audit still passes 56 and fails 24 of 80 routes: two failures at each 90-degree
placement and ten at each 180-degree placement. No physical stair is emitted in
this batch: all three fail authored door support validation and retain projected
navigation with warnings. This does not validate current control apply/reset or
rendered behavior for this batch.

Asset-local inspection finds all inside anchors within their floors, but these
middle anchors outside them (nearest boundary distance in local game units):

| Stair | Door ordinal | Unsupported midpoint distance |
|---|---|---|
| building-156 | 0 | 0.519282 |
| building-156 | 2 | 0.236085 |
| building-154 | 0 | 0.327452 |
| building-154 | 1 | 0.125943 |
| building-134 | 0 | 0.257581 |

Midpoint heights also differ from their outside landing endpoints by up to
0.835567 units. The physical binding requires a supported, matching-height seam;
loosening containment alone would not fix this. Correct asset-owned floor/door
seams before claiming physical traversal for the keep. The compiler now reports
the failing door ordinal, anchor name and world position, and the height error
when applicable. Focused compiler tests verify these diagnostics.

Reproduction: from `level-editor`, run
`node work/map-compile/check-keep-placements.mjs work/map-compile/keep-placements-RegQxL --precise-stairs`.
Pass the printed output directory as an absolute `ROBIN_ASSET_MAP_DIAGNOSTICS`
value to native test `exported_stairs_support_complete_actor_routes --ignored`.
The output directory contains the compiled descriptors and actor-route report.

## Physical stairs in ordinary exports (2026-10-05)

The main compiler now emits physical navigation for compatible planar stairs.
It assembles placed world floors and holes, solids, owner-scoped clearances and
live barriers through the physical region compiler. Motion collision and control
references use that same allocation; retained door ordinals follow exported doors.
Physical anchor resolution uses the world floor while lighting, masks and material
queries retain their projected receiving geometry. Unsupported physical assemblies
warn and retain projected navigation, including the multi-plane fixture.

Six native descriptor fixtures are regenerated from ordinary editor compilation.
Four rotated/elevated stair exports and copied stairs retain matching physical
obstacle IDs and independent controls. Native complete actor routes exercise open,
closed and reset barriers in both directions. Landing binding subtracts overlap
and clips support to the landing side of the equal-height seam; it never extends
support across gaps. This handles rounded landing corners next to exact physical
stairs. The native receiver audit checks physical floor height and footprint
support for physical actors. Synthetic screen-rotation fixtures deliberately keep
their legacy navigation coverage; actual world placement is tested by the editor
exports.

The seven affected compiler suites pass 149 tests, all 63 map-export tests pass,
and the movement suite passes 205 tests (12 ignored). Physical navigation passes
10 tests; level loading passes 40 (three ignored), and patch effects pass 26
(four ignored), including physical routes and projected collision through repeated
apply/reset. Both editor typechecks, focused lint/format checks and the game build
pass. Edge-on surface fitting,
disconnected region assembly, full multi-plane physical navigation, broader
placement audits and rendered traversal remain unfinished. The keep placement
audit has not yet been rerun, and this is not all-map parity certification.

## Local physical stair point dispatch (2026-10-05)

Local point Move requests on an invertible physical stair now resolve their
destination to world coordinates and check its footprint against current floor
and barrier support. They bypass projected pathfinding; emitted movement and
distance-transition orders retain the physical floor and their own world targets.
The normal actor loop performs live physical collision routing. Existing gate
requests retain their explicit world endpoints, including on edge-on floors.

Native tests dispatch plain point movements, ascend and descend between different
heights, and reach the exact world goals without enqueuing projected paths. Other
cases reject an ambiguous edge-on point, an off-floor goal and a goal inside a
closed barrier without relocating the actor. The movement suite passes 205 tests
(12 ignored), and all 33 door-pass tests pass. The game build, focused formatting
and whitespace checks pass.

This does not complete seek/line request creation, mouse destination selection on
edge-on surfaces, physical-distance transition placement, or normal export
integration. The keep placement failures remain unresolved.

## Physical movement source authorization (2026-10-05)

Move/seek instruction entry and synchronous control retranslation now validate
physical stair sources against their world floor and current collision footprint.
They do not send these positions through projected-grid extraction, which cannot
distinguish different heights on an edge-on stair. Unsupported sources warn and
reject without relocation; recovery to nearby supported physical space remains
unfinished.

Native regressions preserve two distinct edge-on world positions, reject a source
beyond the floor without moving it, and check that opening a live stair barrier
changes source authorization. The movement suite passes 203 tests (12 ignored);
patch-effect tests pass 26 (four ignored). The game build and focused formatting
checks pass. General point/seek order generation
and normal physical-stair export integration remain open.

## Physical movement transition handoffs (2026-10-05)

Physical stair orders no longer reject transition animations. Their position
commit uses world-space collision and distance, while the existing transition
lifecycle retains ownership of animation completion and continuation orders.
Reaching a world target clears movement increments but waits for the animation
to finish unless the next order uses the same action. If the animation finishes
first, the copied continuation retains the physical stair and world destination.
Stationary arrival waits retain the ordinary forecast and water-effect updates.

A native edge-on actor-loop test covers a target reached before the transition
ends and a farther target requiring unfinished-distance continuation. Both reach
the exact transition target and then the final destination; the copied order
retains its physical identity and full world coordinates. The complete movement
suite passes (201 tests, 12 ignored), as do all 33 door-pass tests. The game
build, formatting and whitespace checks pass.

This covers explicit physical distance orders with transition animations.
General point/seek order creation, broader animation/seek combinations, soft
repulsion, reciprocal ordinary-neighbour handling and normal export integration
remain unfinished. The keep placement failures remain unresolved.

## Coordinate-aware navigation anchor resolution (2026-10-05)

The main compiler's point-anchor resolver and its diagnostics now use a shared
query that supports projected areas and world-space physical areas. Physical
queries check the actual XY footprint and height, including hole boundaries,
without substituting a snapped screen probe. Projected areas retain the previous
separate exact-height and rounded-contour probes and their existing edge rules.
Physical door validation shares the same boundary-inclusive polygon predicate.

A test feeds emitted edge-on floor geometry into the query and distinguishes
low/high anchors, an interior hole, an unsupported point beyond the floor, a
height mismatch and a nonfinite height even when screen positions coincide.
Another test covers projected rounding, blocked-anchor opt-in and diagnostic
queries without height filtering. All seven affected compiler suites pass
(149 tests), both typechecks pass, and focused lint, formatting and whitespace
checks pass.

The main pipeline does not yet assemble or emit physical stair areas. This
resolves a query assumption needed for integration, not the keep route failures
or complete receiving-feature/material support.

## Physical stair export-frame clipping (2026-10-05)

Physical region assembly now accepts the visible export rectangle. It converts
the screen bounds to linear inequalities on the world height plane and clips
the physical floor before collision assembly. No inverse projection is required,
so an edge-on floor keeps its physical area when its projected line is visible.
The clipped floor retains its height, holes and current control bindings.

Tests cover clipping an edge-on floor, retaining an interior hole and live
barrier, and turning a hole into a boundary notch. In the latter case the
remaining barrier receives the correct new motion-obstacle reference. A sloped
floor test checks the actual world boundary produced by screen-Y limits. Floors
outside the frame, invalid rectangles, disconnected cropped islands and door
anchors left without floor support reject instead of inventing navigation.

All six affected compiler suites pass (147 tests), both typechecks pass, and
focused lint, formatting and whitespace checks pass. The physical region path
still needs wiring into normal export; existing keep placement failures are not
resolved by this component check.

## Crushing across bound stair/landing boundaries (2026-10-05)

Landing collision pieces now retain their owning motion-obstacle index as well
as their state word. A control callback can identify the exact newly appeared
piece after clipping to a receiver, rather than treating all landing collision
as interchangeable.

Closing controls check physical actor footprints across an explicitly bound
stair/landing connection in both directions. A landing obstacle can crush an
overlapping stair actor, and a stair obstacle can crush an overlapping landing
actor. Landing actors must match the bound sector, layer, receiving footprint
and height. The existing unreachable/damage behavior is reused; an unaffected
physical movement order retains its identity and world destination. Only local
paths inside the changed area undergo ordinary retranslation.

Two native tests cover both directions, nearby clear footprints and reopening
without new crushing. They also verify that an unaffected selected physical
order survives a landing control change. Verification passes: 200 movement tests
(12 ignored), 26 patch-effect tests (4 ignored), 10 stair-routing/binding tests
and 40 loading tests (3 ignored).
The game build, formatting and whitespace checks also pass.

Reciprocal neighbour avoidance for ordinary landing movers, soft repulsion,
general point/seek orders, transition choreography, broader placement/state
coverage and normal physical-stair export integration remain open.

## Physical stair collision with landing neighbours (2026-10-05)

Bound landings now retain their runtime sector identity and receiving height
plane. Physical stair collision queries admit a neighbouring actor from another
sector only when its layer, sector, world position and height match one of those
bound patches. A failed landing binding does not provide collision authority.
Ordinary movement retains its existing exact layer/sector filtering.

Physical broad-phase checks now consider each repulsive radius against the stair
footprint expanded by the mover's half-diagonal. A landing actor whose center is
just outside the stair can therefore block an endpoint when its radius overlaps
the supported route. Existing actor target/posture/activity filters remain in
effect.

The new native actor-loop test waits at an endpoint blocked by a lower-landing
actor, resumes after that actor becomes inactive, and reaches the endpoint. A
second case at the same XY but a different height does not obstruct the stair.
Landing-binding checks separately reject wrong sector/layer and positions beyond
the receiver footprint. Verification passes: 198 movement tests (12 ignored),
10 collision tests, 10 stair-routing/binding tests and 40 loading tests (3 ignored).
The game build, formatting and whitespace checks also pass.

This covers a physical stair mover observing a landing neighbour. Reciprocal
handling for an ordinary landing mover, cross-sector crushing, soft repulsion,
broader placement/performance coverage and main export integration remain open.

## Water effects during physical stair movement (2026-10-05)

Physical stair steps now call the same water-particle emission routine as
ordinary movement after committing their world position. The animation-distance
threshold remains strictly greater than two, and the existing splash counter
emits on every third eligible step. Particles retain the actor's current layer
and full world position, including when stair movement has no screen-space
displacement.

A native actor-loop test crosses an edge-on stair with distances at and above
the threshold, on wet and dry material. It checks the counter, emission cadence,
particle kind, layer and exact world position. The fixture faces along the route
so turn slowdown does not change the threshold being tested. The focused check
passes, as does the complete movement suite (197 passed, 12 ignored).
The updated game build, formatting and whitespace checks also pass.

This closes the missing emission call, not complete rendered material/effect
parity. Physical soft repulsion, neighbouring landing-sector collision/crushing,
general point/seek orders, transition choreography and normal export integration
remain unfinished.

## World-space asset surface placement (2026-10-05)

Main compilation now uses a shared placement step for authored surface vertices,
holes and navigation height planes. Placement requires a valid physical plane,
but does not require the projected floor to have area. The main compiler retains
world-space holes alongside its existing world boundary and updates those holes
when authored terrain replaces covered ground. Changing terrain-volume holes use
the same placed geometry, avoiding a separate transform path.

Raised clearances retain distinct physical and navigation-height contours,
including holes; their existing projected footprint semantics are unchanged.
A test places an asset-local sloping floor with a hole at five rotations and two
elevations and passes the result directly to physical region assembly. Exactly
edge-on placements preserve the floor and permanent obstacle identity. Another
test checks raised clearance holes after rotation and elevation.

All six affected compiler suites (144 tests), both typechecks, focused lint and
formatting pass. The main compiler still asks for a projected plane after this
placement step. Receiving-feature binding, area assembly and final lift emission
must be integrated before normal exports can use edge-on physical floors.

## Combined physical stair collision assembly (2026-10-05)

The physical region compiler now joins floor surfaces, slices solid volumes
against the world height plane, applies only clearances owned by the solid's
asset on the same plane, and allocates permanent and changing collision pieces
through the shared state compiler and area emitter. Authored headroom is retained
in the solid's lower plane. Supporting solids that merely touch the floor from
below do not block it. Initial collision contours and control-pair identities
are returned for anchor resolution and runtime binding.

Ordinary static and changing collision compilation now shares the height-slice
routine with this path. Projection and polygon precision remain chosen by each
caller, so ordinary export retains its fixed-point clipping behavior.

A combined placement test covers five rotations, elevation and two headroom
settings. It checks floor holes, a cleared wall, a separately owned post that
must survive the clearance, a raised beam's intersection with the slope, a
supporting foundation, fractional changing barriers and independent copied
controls. Physical pieces reference emitted state records; exact edge-on floor
projection remains valid.

All five affected compiler suites (142 tests), both typechecks and focused lint
pass. This is still compiler integration groundwork: ordinary surface placement
and receiving-feature binding depend on projected geometry, so the normal export
does not yet invoke the physical region assembler. Keep placement failures remain
unresolved.

## Physical changing-barrier compilation (2026-10-05)

The movement transition compiler now shares state allocation and hole
triangulation between projected and physical coordinate paths. The physical
entry point accepts world contours/planes, slices vertical volumes without
projecting them, and retains fractional contours until area emission. It does
not discard barriers merely because they cover less than one projected pixel.
Ordinary compilation retains its existing quantization and preserved-boundary
behavior.

An end-to-end compiler test passes changing barriers into the physical stair area
emitter at five rotations, with elevation, including an exactly edge-on floor.
It verifies independent control pairs, initial/applied state words, the retained
area of a 0.1-unit-wide barrier, and a holed barrier's triangulated area through
the emitted physical-to-motion obstacle references. A separate volume test
checks that a horizontal height range cuts the expected world-space strip of an
edge-on stair even though the entire strip projects onto one line.

Both new tests, all four affected compiler suites, both typechecks, focused lint
and formatting pass. Main export integration remains unfinished; these results
do not resolve the existing keep placement failures.

## Retained world planes for collision compilation (2026-10-05)

Placed navigation surfaces now retain their world height plane and footprint.
Terrain clipping updates the retained footprint, and static-solid slicing uses
the original plane instead of fitting it again from a projected outline. The
broad-phase bounds include both the authored footprint and the integer outline's
extent so rounding does not exclude nearby collision.

Navigation pieces carry the retained plane through assembly into changing-volume
slicing. Older standalone callers can still reconstruct it when not provided.
This prepares the main pipeline for physical stair compilation; projected surface
assembly and final emission remain unchanged, and the keep failures remain open.

Verification: all four affected compiler suites pass, both app/pipeline typechecks
pass, and focused lint and formatting checks pass.

## Physical stair area emission and native contract fixture (2026-10-05)

The world-space compiler now emits a complete motion-area fragment together with
its physical navigation. Permanent holes and changing collision pieces receive
one ordered set of obstacle identities; each physical piece references the same
index and retains the corresponding runtime state word. Projection happens only
after this allocation. Collapsed projected contours retain their ordered vertices
instead of being simplified into invalid two-point polygons. Out-of-range game
coordinates and invalid state words are rejected.

`shared/test-fixtures/physical-stair-area.json` is checked against compiler output
in the editor tests and embedded directly by the Rust edge-on traversal fixture.
The native actor tests therefore consume emitted area/navigation data, rather
than maintaining a handwritten duplicate of the stair contract. All twelve
focused native checks pass, including complete gate routes in both directions,
barrier closure/reopening, landing support and nearby-actor detours.

The main asset compiler still assembles its intermediate surfaces and collision
in screen space. Switching those stages to the physical frame remains necessary
before normal editor exports can use this emitter. The 24 keep failures remain
unresolved; this fixture does not establish full-map export parity.

Verification: 124 affected editor tests, both typechecks, focused lint, formatting,
whitespace checks and the updated game build pass.

## World-space stair compilation primitive (2026-10-05)

The shared compiler contract now describes `physical_navigation`. A separate
world-space compilation step joins coplanar placed stair surfaces before screen
projection, preserving physical floor area, holes, motion-obstacle identities and
ordered world door anchors. It rejects disconnected floors, inconsistent heights,
invalid obstacle identities and door anchors without floor support. Permanent
holes are returned separately for allocation of their motion-obstacle IDs.

The tests cover an exactly edge-on floor and ten rotated/elevated placements,
including joined surface parts and a retained hole. Opposite door anchors retain
different world heights even when their screen coordinates coincide.

This primitive is not yet called by `compileAssetGameplay`. That pipeline still
assembles stair surfaces, solid slices and changing barriers in screen space and
can reject an edge-on floor before serialization. It must use the physical frame
through those stages and allocate the returned holes/state links before emitting
the new field. Ordinary exports and the 24 failed keep routes are unchanged.

Verification: 122 affected compiler tests pass, including the three new tests;
both editor/pipeline typechecks, focused lint, formatting and the game build pass.

## Physical stair gate-route dispatch (2026-10-05)

Gate-route assembly now retains the destination door identity for an approach
inside a physical stair. Movement dispatch resolves that door's world endpoint
and emits a physical order through the normal movement-element lifecycle. It
does not ask the projected pathfinder to repair a source on a collapsed floor.
Physical collision and current barriers remain checked before each actor step.
Ordinary point-only approaches retain their existing representation.

A native fixture constructs a normal two-door gate route between the lower and
upper landings of an edge-on stair. Both directions complete through sequence,
path and actor ticks, with many intermediate world-height samples proving that
coincident screen endpoints do not skip the stair. Two further journeys close a
full-width physical barrier mid-route, verify eight stationary ticks, reopen it
and reach the opposite landing. The barrier's projected polygon is collapsed too.

Compiler emission is still missing, so this does not resolve or supersede the
24 failed keep routes. General point/seek requests on physical floors, transitions,
cross-sector neighbour/crushing behavior and full movement effects remain open.
Verification: 196 movement tests pass (twelve external-fixture checks ignored),
as do 33 door-pass tests and 26 patch-effect tests (four ignored). Formatting and
whitespace checks pass. The updated game build passes:
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1`.

## Physical stair landing binding and complete door passes (2026-10-05)

Loading now binds landing support from each door's current motion area and actual
projection receiver. Navigation is converted onto that receiver's height plane
and clipped to its real footprint; the receiver plane is not extrapolated over
an entire multi-height area. Implicit ground uses its zero-height plane. Binding
requires matching doorway heights and a shared, height-matched floor edge.
Missing, overlapping, singular or incompatible support emits a warning and does
not provide extra walking space. Holed collision clips are also still unsupported.

Landing holes and collision are retained separately. Collision reads its own
motion area's current state, including restored state; it does not inherit the
stair area's switches. Physical actor steps now use the landing-aware query.
Complete actor-loop passes enter and leave both doors of the edge-on fixture,
retaining world height and the outside receiving plane. A separate fixture closes,
opens and restores a landing barrier under the stair footprint.

This integration exposed a re-planning failure at rounded obstacle tangents.
Visibility now uses the same f32 coordinate-error budget as endpoint seating;
the nearby-actor detour and a captured tangent-position regression both pass.

Automatic between-door route creation and compiler emission remain unfinished.
Crushing across landing-sector boundaries, neighbours across those boundaries,
soft repulsion, transition choreography and movement effects still need work.
The 24 failed keep routes have not been rerun or resolved. Full-scene performance
of physical routing also remains unverified.

Verification: eleven focused physical-stair tests, ten routing/binding tests,
195 movement tests (twelve ignored), 40 level-loading tests (three ignored),
33 door-pass tests and 26 patch-effect tests (four ignored) pass. Formatting and
whitespace checks pass. The updated game build passes:
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1`.

## Physical stair landing-support routing (2026-10-05)

A new route query separates support for the actor's footprint from ownership of
its center position. It unions supplied landing polygons with the physical stair,
erodes support by the actor's effective rectangular footprint, expands live solids
by that footprint, and intersects the resulting center space with the stair floor.
A visibility graph through this polygonal free space preserves holes and connects
the requested endpoints. Thus a landing can support feet overhanging a short stair
without allowing a detour onto that landing using the stair's extrapolated height.

Endpoint normalization is limited to f32 coordinate rounding error. The rotated
seam cases retain the caller's exact endpoints; a real gap in support is rejected.
Tests also reject a route around a full-width stair barrier even when broad
landings would otherwise permit that shortcut, and sample actor clearance along
a route around a partial obstacle.

This API is not yet called by normal actor movement. The compiler/runtime still
need to supply connected, height-matched landing polygons and their live collision
ownership. It does not yet fix the 24 failed keep routes or complete door crossings.
All eight physical-routing tests pass, including ten directed seam queries at
five rotations, missing/gapped support, forbidden off-stair detours and sampled
clearance around a partial solid. Formatting and whitespace checks pass.
The updated game build passes:
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1`.

## Physical stair door identities and receiver handoffs (2026-10-05)

Stair door translation now retains the world destination for the inside walk.
It resolves endpoints by owning lift and local door ordinal, so lower and upper
doors remain distinct even when their projected positions coincide. Outside
walks continue to use the landing's normal movement path and animation.

Entry binds the physical floor at the shared world-space door midpoint. Exit
restores the outside receiver and preserves that midpoint before the next step.
The edge-on fixture checks both endpoint heights and that a subsequent outside
step follows the correct landing plane. Translation checks cover both directions
at both doors. These are focused order/callback tests, not complete crossings:
landing-footprint support and automatic between-door routes remain unfinished.
Compiler emission remains disabled; the 24 failed keep routes are still open.

Verification: all nine focused physical-stair checks pass, as do 33 door-pass
tests and 193 movement tests (twelve external-fixture checks ignored). Formatting
and whitespace checks pass. The updated game build passes:
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1`.

## Physical stair actor execution (2026-10-05)

Explicit physical distance orders now execute in the normal actor update loop.
Orders retain a stair identity and world destination through native encoding.
Animation timing and completion use the existing movement handlers, while route
queries, stepping, arrival and the final position snap use physical coordinates.
Every step reads current control state and nearby actors' hard repulsive regions.
Neighbour filtering uses world ground positions so distinct actors at the same
screen point are not accidentally ignored. Forecasts follow committed motion.

Physical orders retain their destination when a control changes; replacing them
with an ordinary projected path lost both height and route identity. Appearing
obstacles test crushing against the physical footprint. Physical floor ownership
also bypasses projected elevation reattachment during the step. The loader omits
screen-space passage bonds for physical doors, whose midpoint and inside approach
may legitimately coincide in projection.

Actor-loop fixtures load a genuinely edge-on floor with real lower/upper landing
planes. Explicit orders traverse it both ways over many animation ticks while
screen Y remains constant, including position-state restoration mid-route. Other
fixtures exercise an equally projected neighbour, control closure/reopening during
motion, and crushing inside the physical obstacle. These fixtures do not yet
exercise automatic player route requests or door entry/exit.

Remaining integration: compiler emission, automatic physical route/order creation,
door handoffs, landing-footprint support, physical transition choreography, soft
repulsion and shared water-particle emission. Normal editor exports still use the
existing route pipeline; the 24 failing keep routes have not been resolved.
Native snapshot version is now 10 and replay schema 60 because order binary
layout changed. Disk-save version remains 97; the new optional order field has a
deserialization default.

Verification: movement passes 191 tests (twelve external-fixture checks ignored),
anti-collision passes ten, patch effects pass 26 (four ignored), snapshots pass
54, replay tests pass 29 and order tests pass thirteen. The complete level-data
suite passes 80 (seven ignored), and replay-format passes 26. Formatting and
whitespace checks pass. The updated game build passes:
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1`.

## Physical stair descriptor and live collision binding (2026-10-05)

An optional lift `physical_navigation` definition retains a world-height plane,
ground-coordinate boundary, physical collision polygons and ordered world-space
door anchors. Collision pieces reference the existing motion area's obstacle
indices. Validation rejects omitted/missing obstacle identities, malformed
polygons, mismatched door counts, nonfinite coordinates, inconsistent projected
anchors and inside/middle anchors outside the stair plane.
Low/high identities must agree with physical endpoint heights, and inside/middle
anchors must lie within the physical boundary. Screen-space approach adjustment
leaves these physical definitions unchanged; otherwise it would rewrite their
door points after validation against the placed floor.

Runtime loading binds these definitions to their motion layer and area. Physical
route queries consult the normal pathfinder's current state word, so they do not
require a duplicate switch-state table and can use restored state. The regression
fixture loads through normal engine construction and exercises a full-width
barrier's initial/open/closed states and restored pathfinder state. Validation is
also exercised through descriptor loading.

Compiler emission and actual actor traversal remain unfinished. In particular,
the new metadata does not yet authorize collapsed projected motion polygons,
provide landing-footprint support, or change door orders/receiver handoffs. The
24 failed keep routes remain unresolved.

Verification: all three focused physical-stair tests pass. The movement suite
passes 187 tests (twelve external-fixture checks skipped), level loading passes
40 (three skipped), physical collision routing passes five, and the complete
level-data suite passes 80 (seven skipped). Formatting and whitespace checks pass.
The first game build was terminated with exit 143 (SIGTERM), without a compiler
diagnostic. The single-job incremental retry passed:
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1`.

Reproduce the focused integration checks with
`RUST_MIN_STACK=33554432 RUSTC_WRAPPER= cargo test -p robin_engine -j1 --lib physical_stair -- --test-threads=1`.

## Physical stair sprite execution and receiver entry (2026-10-05)

The sprite motion API now accepts an explicit physical destination while sharing
its animation/frame progression with ordinary motion. It initializes the cached
3D increment forward, distinguishes world-space arrival from coincident screen
endpoints, and rejects a changed physical goal under the same order identity.
The position interface can attach a receiver at a supplied ground point, computing
height forward instead of dividing by the screen-projection determinant. Existing
screen-motion and receiver APIs retain their behavior.

The test binds a receiver with height plane `z = y + 40`, animates WalkingStairs
in both directions using the script's actual per-frame distances, and advances
the physical route while its screen position stays fixed. It round-trips the
position interface mid-route, checks exact endpoint arrival and rejects a
same-order-ID change to a different physical endpoint with the same projection.
All 37 sprite tests, 46 position-interface tests and 185 movement tests pass
against the final source; twelve movement tests requiring external exports or
animation resources are skipped. Formatting, whitespace checks and
`RUSTC_WRAPPER= cargo build -p robin_rs --bin robin -j1` pass.

The physical route owner must still perform collision checks and commit world
positions. Compiler metadata, route-order creation, door handoffs and actor-loop
dispatch remain unfinished; ordinary editor exports still use the existing stair
path. These APIs do not by themselves resolve the 24 failing keep routes.
The descriptor validator still requires every lift motion area to have at least
three projected polygon points. Door translation stores screen destinations, and
door transitions snap through screen-coordinate receiver lookup. Integration must
carry physical endpoint identities through those boundaries as well as the actor
movement loop; allowing a collapsed polygon alone would not fix traversal.

## Physical stair collision routing foundation (2026-10-05)

`robin_engine::stair_navigation::StairRouteGeometry` builds an independent
ground-coordinate grid and visibility graph from a physical boundary and the
current active obstacle polygons. It reuses the engine's footprint/corridor
queries and pathfinder. Negative world positions are translated into a bounded
local grid; returned endpoints retain their caller coordinates. Malformed
geometry is an error, while valid unreachable requests return no route.

The route boundary must include actual connected landing support. The router
does not enlarge short stairs or invent terrain. It verifies endpoint support,
footprint clearance and every returned segment. Obstacle state changes require
a fresh geometry snapshot; this API does not yet bind to runtime control events.

All five physical-routing tests and 22 pathfinder tests pass, as do formatting
and the game build. The new cases
cover an edge-on stair detouring around solid geometry, sampled polygon-based
footprint checks independent of the grid, ten directed routes at five rotated/
translated placements, a wall that blocks until removed, unsupported endpoints,
invalid geometry, a tiny obstacle wholly inside the swept corridor and a short
stair that needs actual landing support.

This remains groundwork for physical stair traversal. The compiler format,
door handoffs, current height/receiver ownership and actor movement loop still
need integration. It does not resolve or supersede the 24 failing keep routes.

## Physical stair coordinate and stepping foundation (2026-10-05)

`robin_level_data::stair_navigation` now provides a validated physical height
plane and forward world/screen projection, plus distance-bounded advancement
along a physical route segment. It does not invert the screen projection or
use screen displacement to determine arrival. Tests walk both directions along
an exactly edge-on segment whose screen endpoints coincide, preserving height
progress and exact endpoint arrival. Further tests cover projection reversal,
near-singular slopes, serialization and nonfinite/overflow rejection.

The level-data suite passes 80 tests with seven ignored, and the game build
passes. This API is groundwork:
the editor does not emit a physical stair-navigation field yet, and actor
movement does not consume it. Physical collision/landing support, door handoffs,
movement-loop integration and native failing-route reruns remain required.
The existing 24 failures are not reclassified as passing by these unit tests.

## Stair footprint projection and singular placements (2026-10-05)

`refinement/audit-stair-footprints.mjs` audits the authored stair planes using
the editor's placement transform. It compares necessary corner containment for
the 12-by-6 test footprint in physical XY, for the current unchanged screen box,
for the plane-projected parallelogram and for its axis-aligned bounding box.
It asserts the affine area invariant and independently verifies analytically
derived edge-on rotations. The output is
`work/map-compile/keep-placements-RegQxL/physical-footprint-audit.json`.

All fifteen placement checks and six edge-on checks pass. At 180 degrees, the
large gallery stair retains about 976 square units of possible physical foot-box
centers, but none for the unchanged screen box. Its projection determinant is
0.05648; projecting the footprint with the floor retains about 55.12 square units
of centers. Using that parallelogram's axis-aligned bounds still leaves zero.
The west-tower stair shows the same bounding-box failure at 180 degrees.

The three authored stair planes each have two rotations where screen projection
collapses them to a line. The gallery examples are approximately 122.449 and
173.942 degrees. Physical areas remain 1,899, 277 and 1,861 square units for the
three surfaces, respectively; projected areas at their singular rotations are
below 3e-11. Consequently, neither coordinate precision nor transformed screen
footprints can provide arbitrary-rotation traversal in the current representation.
Navigation must retain a nonsingular physical/virtual coordinate frame and map
actor positions, door approaches and queries to rendered coordinates separately.

The shortest gallery stair has no full physical-box center at 90/270 degrees;
landing overlap must be considered as well. This audit ignores holes, solids and
door connectivity and is not a route test. No runtime behavior or asset geometry
has been changed by it; the 24 of 80 native route failures remain unresolved.

## Published draft wood bridge and geometric review (2026-10-05)

The bridge is published with an explicit warning that textured actor compositing
remains unverified. Its gameplay matches the native-tested headroom candidate;
only the draft warning changes. The pinned review catalog records both descriptor
hashes. Publication backups and the scene-pin audit are in
`work/map-compile/published-imported-bridge-headroom`; all ten saved scenes reopen.

`refinement/review-imported-bridge-collision.py` renders the pinned mesh alongside
the actual authored volume faces and deck surfaces in three orthographic views.
The inspected `imported-bridge-deck-X16cpf/collision-review.png` shows conservative
timber proxies, retained railing/cross-brace gaps and foundation extensions below
the short support feet. It is a geometric review, not textured in-game validation.
No mesh or textures changed. The library now has 34 missing gameplay definitions.

Reproduce the published placement checks from `level-editor` with:

```sh
node refinement/stage-imported-bridge-gameplay.mjs --structure --published
```

This reconstructs the authored geometry from the pinned mesh, compares its JSON
representation to the installed gameplay, then compiles the installed definition.
JSON comparison normalizes negative zero exactly as publication does. The command
requires the hull files produced by `author-imported-bridge-supports.py --structure`.
The published run `work/map-compile/imported-bridge-deck-BFZSPh` passes all 30
directed native actor routes, 75 blocked points and 140 sight/projectile probes.
All ten descriptors differ from `X16cpf` only in their single draft warning;
the placement script also passes all ten landing-height rejection checks.

## Authored movement headroom and bridge detours (2026-10-05)

Permanent gameplay volumes can now author `movementHeadroom` in world game-height
units. After placement, navigation subtracts that height from the solid's bottom
plane when deriving walking restrictions. Physical sight/projectile geometry is
unchanged, and standing on the solid's top remains possible. The default is zero;
no mission character profiles are compiler inputs. Spline copies retain the value.
Tests cover rotation, elevation, sloping floors, standing on top, raising a solid
away from independent terrain, repeated spline deformation and invalid values.

The bridge review recipe specifies 80 units. The reproducible candidate
`work/map-compile/imported-bridge-deck-X16cpf` passes 30 directed native actor
routes, 75 blocked points and ten landing-height rejection checks across five
rotations. Ten routes cross the deck; twenty foundation routes detour around
the now-blocked low structure. The separate native ray test passes all 140
sight/projectile checks, including gaps below the wood. Ray clearance is not
walking clearance. Earlier fixture endpoints at 37 degrees shared one receiver
and failed the crossing helper's precondition; the final route crosses distinct
receivers without changing that helper or compiled geometry.

All 134 affected compiler, spline and collision-subtraction tests pass, as do
app/pipeline typechecks, focused lint, formatting and the production editor build.
The candidate remains unpublished pending rendered collision/actor review; these
checks do not establish full map parity or posture-dependent crawling behavior.

## Bridge sight/projectile gaps and body-clearance review (2026-10-05)

`work/map-compile/imported-bridge-deck-y6Ojxi` adds explicit world-space ray
probes without changing any of the ten previously tested geometry descriptors.
The new ignored native test
`exported_geometry_preserves_authored_sight_and_projectile_gaps` loads the
compiled map and queries its active obstacles. It passes 140 checks: seven
probes, five rotations, both directions and separate sight/projectile flags.
Deck thickness, upper rails, posts and cross-braces block; the selected railing
gap, open underpass and space below a brace remain clear. The complete result is
`ray-probe-report.json`. Focused lint, Rust formatting and the game build pass.

These point rays and movement-footprint tests do not establish body clearance.
The library's upright character previews extend substantially above their foot
hotspots: sampled profile bounds reach 53 pixels for Robin Town and 76 for
Little John. The blocking brace probe is only about 42 game-height units above
the foundation. Navigation currently derives solid intersections at the walking
plane, so a clear foot route can still pass beneath wood that intersects an
upright body. The bridge remains unpublished until its walking restrictions or
asset-authored headroom policy account for that difference. No mission cast or
character profile should become a runtime map-compilation dependency.

## Collision subtraction recovery and bridge structure candidate (2026-10-05)

The fitted bridge structure exposed a floating-point sweep failure while
subtracting a solid from navigation. The compiler now preserves successful
subtractions exactly and retries failed ones using the existing fixed-point
geometry operations. No solid is dropped. The captured regression checks
158,400 point occupancies against independent subject-minus-cut membership,
including retained free space and existing holes. All 120 focused compiler,
clipping and subtraction tests pass, along with editor/pipeline typechecks,
focused lint and the production editor build.

The structure candidate is generated with the same two authoring commands below,
adding `--structure` to each. It fits 150 oriented wood-piece proxies, splitting
long rails along their arched profile. Eighteen deck prisms use the mesh's paired
bottom vertices, retaining its varying thickness. These proxies are deliberate
collision approximations, not a claim of exact mesh occupancy. Coplanar face
merging reduces the complete candidate from 3,499 to 1,527 volumes while the
per-hull volume checks still pass. Numerical dust below 1e-7 square local units
is discarded before it can create a degenerate height plane.

`work/map-compile/imported-bridge-deck-SMVEnH` compiles strictly at five rotations
and passes thirty directed native deck/underpass routes and all sixty blocked
support-foot probes. Ten mismatched landing cases have disjoint wood/terrain
navigation references. Matching cases share a region used for actual traversal;
the fixture no longer assumes that adding collision cannot create extra regions.
A small isolated 0.5-square-unit walking fragment was observed at 37 degrees
before coplanar face merging, so region counts alone cannot certify these routes.
The candidate remains unpublished pending direct sight/projectile queries and
visual/body-clearance review; the library still has 35 missing definitions.

## Imported wood bridge deck candidate (2026-10-05)

The support follow-up is reproducible from pinned mesh data with:

```sh
cd level-editor
python3 refinement/author-imported-bridge-supports.py
node refinement/stage-imported-bridge-gameplay.mjs --supports
```

The Python authoring step requires NumPy and SciPy. It finds twelve disconnected
inclined supports and builds a convex hull for each. Only foot vertices below
two scene units are extended to foundation Z=0; the largest extension is
1.397324 scene units. These are deliberate collision hulls, not an assertion of
exact mesh occupancy. Their vertical decomposition produces 590 obstacle pieces;
integrated piece volume matches each hull within a relative tolerance of 1e-7.
Each volume carries wood material and solid/opaque/mouse flags. No level data is
read, and the scripts do not publish the result.

`work/map-compile/imported-bridge-deck-GcCB3a` passes thirty native directed
routes: ten complete deck crossings and twenty underpass crossings across five
rotations. All sixty support-foot sample positions are blocked. Ground-route
endpoints are inside terrain triangles; an earlier fixture put them on ambiguous
shared boundaries and failed only the final receiver-identity assertion.
The checked-in scripts reproduce all ten descriptors and the candidate gameplay
exactly in `imported-bridge-deck-g4Js4g`; focused lint passes. Landing-height
rejection checks also still pass. Railings, deck thickness and cross-braces remain
unauthored, so these tests do not certify complete under-bridge body clearance,
projectile/sight collision or publication readiness.

A direct body-conversion experiment found open boundary seams in 130 of the
mesh's 131 connected pieces. Closing 260 boundary loops and pairing projected
top/bottom faces produced 5,802 cells, with overlapping intervals and tiny
degenerate pieces. The candidate in `imported-bridge-deck-4X1MMW` fails compiler
height-plane construction and is not published or used by the checked-in tool.
Its `body-volume-review.json` records summed-cell versus signed-mesh volumes;
these are not equivalent occupancy measures when cells overlap. The next body
authoring step needs reviewed, compact collision shapes preserving the arched
deck, rail openings and cross-braces, rather than treating that conversion as a
usable definition. Experimental scripts remain under `work/map-compile` as
`author-bridge-closed-bodies-experiment.py` and
`stage-bridge-closed-bodies-experiment.mjs`.

Mesh review identifies eighteen top-facing triangles spanning nine arched deck
panels, excluding the lower beams and railings. Their pinned face selection is
recorded in `refinement/catalogs/sketchfab-long-wood-bridge-deck-review.json`.
The staged definition uses local game coordinates, wood receiving material,
one multi-plane navigation region and two end sockets. It compiles strictly at
0, 37, 90, 180 and 270 degrees. The review script is
`work/map-compile/stage-imported-bridge-deck.mjs`; the resulting descriptors,
source pins and routes are in `work/map-compile/imported-bridge-deck-O5SOR0`.
The subsequent `imported-bridge-deck-MiYngY` batch also checks landings one game
unit above and below each of the five placements. All ten mismatches retain
separate deck and terrain navigation regions; matching landings form one region.

Native endpoint tests pass ten directed complete crossings. Native receiving
seam tests pass another 754 sampled crossings. The fixture terrain is at the
deck-end height, so these checks verify deck/landing traversal, not an underpass.
Support and railing collision, projectile/sight obstruction and under-bridge
clearance remain unauthored. The candidate explicitly warns about those gaps
and is not published; the missing-definition count remains 35.

## Keep stair precision and projected actor clearance (2026-10-05)

The clearance experiment `work/map-compile/keep-placements-FgNcOp` adds complete
stair-surface clearances without widening the walking surfaces. It still fails
the same 12 of 68 native routes. Its `stair-clearance-audit.json` intersects the
candidate center regions for all four corners of the test actor's 12-by-6
movement box. Every failed stair region has an empty intersection: no position
can contain that box in the projected walking polygon. This necessary-condition
check does not prove routes through polygons with a nonempty intersection.
The failures therefore cannot be fixed solely by changing the actor's route.

Preserving fractional stair coordinates through clipping separately fixes the
180-degree assembly disconnection. `work/map-compile/keep-placements-RegQxL`
uses only that precision setting, without the experimental full clearances.
All eight placements retain three stairs and pass native construction and the
available control's apply/reset. The native traversal audit still fails 24 of
80 routes: the extra twelve tested routes are on the formerly omitted stair,
and all twelve fail. The previously passing 56 routes continue to pass. This
is an assembly correction, not a solution to projected actor clearance.

The three precision settings are published on `derby-keep-central-gallery`
and `derby-keep-west-tower`, using the compact pinned recipe
`refinement/catalogs/derby-keep-stair-precision.json`. The publication helper
supports reviewed per-surface precision settings without copying megabytes of
unchanged mask geometry into a recipe. Its four tests and pipeline typecheck
pass. `work/map-compile/keep-stair-precision-3DHB3d` contains the reviewed before/
after Derby exports and transactional publication backups. Derby's complete
compiled geometry is exactly unchanged; both snapshots pass all 28 native stair
routes and five control apply/reset checks. Scene pins are refreshed without
moving placements. All ten saved scenes reopen with current descriptor pins
(`scene-pin-check.json`). A subsequent uncalibrated Wychford comparison failed
exact descriptor equality: precision changed a receiving polygon linked to
projection area `[314, 15]`. That assertion stopped the batch before it saved
the Wychford snapshots or reached Derby; it supplies no new native traversal
result. The geometry difference still needs assessment with calibrated spline
assets and preserved before/after descriptors. Wychford geometry equality is not
claimed. The composite keep remains unpublished, and no ZIP is rebaked.

The subsequent calibrated comparison in
`work/map-compile/wychford-wall-calibration-EFEmVR` preserves both descriptors
before comparing them. Only obstacle 21,321's points differ, with its projection
area `[449, 22]` unchanged. All other geometry and descriptor fields, and the
warning lists, are exactly equal. Polygon XOR measures about 0.000658 square
game units in physical XY and 0.001043 square map units in projected coverage.
The descriptor retains 21,323 sight obstacles. The current snapshot passes
native construction and all three control apply/reset checks. Both snapshots
also pass all four available directed stair routes, with no failures. This small
geometric difference is not an exact round trip, nor by itself evidence of a
gameplay regression; native traversal checks are recorded separately.
The current calibrated snapshot's full receiving-seam audit subsequently
completed: 17,482 directed actor crossings pass across 8,741 eligible receiver
pairs. Its complete report is
`wychford-wall-calibration-EFEmVR/current-check/actor-receiver-crossing-report.json`.
This covers sampled initial-state crossings, not every route or control state.

## Composite keep placement and best-effort failures (2026-10-05)

The revised candidate `work/map-compile/keep-reassembly-yEL2X5` explicitly
preserves each component's choice of implicit solid versus authored-contour
movement collision. Concatenating components without that distinction silently
changes collision. The comparison in `keep-reassembly-comparison-NFpDXX`
preserves all 17 ordinary doors, 14 building groups and 12 lifts, including
door coordinates/permissions, room memberships and traversal endpoint identities.
Runtime indices reorder. Both the original component assembly and the candidate
pass all 28 directed native stair routes and five control apply/reset checks.

`work/map-compile/keep-placements-wVlmPz` tests new placements on authored terrain
at rotations 0, 37, 90 and 180 degrees and elevations 0 and 40. All eight exports
now construct and pass their one available control's apply/reset checks. The
stair audit is **not green**: 56 of 68 directed routes pass. Both 90-degree
placements fail two routes; both 180-degree placements fail four routes and omit
one additional disconnected stair. Failed actors stall inside narrow traversal
regions. This needs geometry/runtime diagnosis before publication; no asset
definition or saved placement has been changed by these staged checks.

The placement batch exposed two best-effort export aborts, now fixed. Collapsed
character/projectile mask boundaries warn and lose only the unusable rule;
independent view/obstacle rules remain. Masks with no surviving rule are omitted
without dangling state references. A collision-split lift region raises a typed
placement error so the existing retry omits its complete traversal assembly while
retaining collision and independent gameplay. Strict export retains both errors.
Compiler, mask and navigation suites pass 127 tests; app typecheck, focused lint
and the production editor build pass. These fallbacks are omissions, not parity.

## Standalone gameplay frame migration (2026-10-05)

`pipeline/src/translate-gameplay-frames.ts` provides an authoring-only conversion
between translated part-local frames. It moves surfaces, physical/navigation
heights, clearances, doors, traversal sockets, volumes, masks and receiver probes,
materials, lighting, sound positions, scenery and state-control geometry together.
Relative directions, distances and permissions remain unchanged. Spline metadata
requires recalibration; a shared placement height requires a shared vertical shift.

Sixteen tests pass, including complete compiled-geometry comparisons for fifteen
fixtures before/after migration: slopes, clearances, receivers, masks, changing
stairs, interiors, jumps, controls, materials, sounds and rotated scenery. The
pipeline typecheck and focused lint pass.

`work/map-compile/keep-reassembly-5HGm6U` stages the composite Derby keep from its
four gameplay-bearing components. Every one of its 71 physical parts matches
the corresponding component after translation. The combined gameplay validates
after separating source-local navigation labels and appearance IDs. This is an
unpublished authoring candidate: collision ownership, appearance resources and
native traversal/state comparisons remain required before publication. The
library's missing-definition count is unchanged.

## Reopening barriers during climbing (2026-10-05)

All five native `changing_climb` tests pass, including 72 reopening cases across
ladder and both wall-top passage types, four rotations and both directions.
The test closes the barrier after the actor enters the climb area and starts
its movement instruction, then reopens through ordinary control activation.
Reopening before path failure completes the route. Reopening while a failed
request waits, or after its timeout, does not automatically restart that route.
Failed requests retain the existing 100-frame timeout; forced reset deliberately
does not notify actors to replan. These checks use complete character animation
data but do not verify rendered traversal or every possible placement.

## Library-wide missing-definition audit (2026-10-05)

The saved-scene audit does not cover every asset available for new maps.
`work/map-compile/library-missing-gameplay-20261005.json` records the library
index hash, descriptor hashes, part counts and physical-part counts for all
35 missing definitions among 1,288 indexed assets. All 35 are absent from both
their index entries and descriptors, rather than merely being stale index data.
They include fourteen Croisement groups, the composite Derby keep, Leicester's
older ground background, five Nottingham entries, an imported wood bridge and
thirteen York entries. None is placed in the ten saved scenes. These require
individual authoring/ownership review; buildings and bridges cannot receive
empty scenery declarations merely to remove export warnings.

The follow-up `work/map-compile/library-missing-gameplay-owners-20261005.json`
uses every indexed descriptor path, including assets directly under the asset
root. It pins both missing and candidate-owner descriptors. Thirty-two missing
assets have all their part names represented in gameplay-bearing replacements:
the Derby keep in four components, the fourteen Croisement fragments in five
state assemblies, four Nottingham assets in combined/remainder assets, and
thirteen York assets in market-state/remainder assets. The imported bridge,
Leicester ground background and Nottingham terrain ground have no such match.

All fourteen Croisement fragments participate in old/new sight-obstacle sets
of map controls. They cannot simply be restored as always-active collision.
The component matches are authoring candidates, not proof of equal geometry or
frames. Restoring standalone definitions must preserve their own features while
separating unrelated assembly parts and control dependencies. No definitions
were published or missing-definition warnings suppressed by this audit.

## Published Lincoln static-prop definitions (2026-10-05)

Five recently added scenery assets had no gameplay declaration: the barn tool,
longhouse loose log and path poles, pond loose plank and south-bank stake.
They now explicitly declare no collision, walking surfaces or doors. Their
models still participate in the normal color/depth bake. The reviewed edits are
in `refinement/catalogs/lincoln-static-props-gameplay.json`.

The authoring review used the Lincoln map descriptor with SHA-256
`a09299f83f8f5d5de59aa73cacf058d8c4b82c69c6c8fe0798011118e051e55d`.
No independent sight volume belongs to these five props. Overlapping physical
records 15/16 belong to the open barn, 19 to its cart, 24 to the longhouse and
36 to the separate longhouse props. The nearby movement contours similarly
describe the cart and longhouse assembly; they must not be duplicated onto the
tool or loose poles. The log and stake have no overlapping physical-volume
bounds. These definitions do not claim neighboring masks or lighting regions.

`work/map-compile/lincoln-static-props-H7noaD` contains the staged export,
before/after warnings, publication backups and scene audit. Lincoln's complete
compiled geometry is unchanged, excluding warnings; exactly the five missing
definition warnings disappear. Twenty fresh terrain assemblies (five assets at
four rotations, each with a separate copy) retain the bare terrain geometry
exactly. Native construction and all eleven Lincoln control apply/reset checks
pass. No mission entities are added.

Publication refreshes Lincoln's five descriptor pins. All ten saved scenes
reopen; the fresh published Lincoln export equals the native-tested candidate.
No placed asset in those scenes lacks a gameplay definition. Wychford still
lists the unused `derby-great-keep` reference without gameplay; that reference
has no placed instance and contributes nothing to compilation. Existing draft
coverage warnings remain, and no baked archive was regenerated by this change.

## Published watermill collision and physical platform clearance (2026-10-05)

The watermill now owns body movement collision. Its platform clearance and
receiver anchor use physical coordinates plus a local `navigationHeight` for the
foundation plane. Transforming before projection avoids the diagonal-rotation
entrance failure recorded below. Clearances retain their physical slope and holes,
cut only their owner's solids, and do not invent a floor or bridge height gaps.
Invalid heights and incompatible receiver segments are rejected. Spline exports
explicitly reject these separate clearance heights, or warn and retain collision
in best-effort mode.

The compiler/export/spline suites pass 189 tests; app typecheck, focused lint and
the production editor build pass. Regression cases cover sloped clearances at seven rotations and two
elevations, holes that must remain blocked, and mismatched navigation heights.
`work/map-compile/watermill-placement-F6AfOv` contains 24 new placements at eight
rotations and three elevations, retaining all three entrances, one jump pair and
both masks. Native checks pass 48 directed platform crossings and 24 blocked
body-interior points. Generated integer-grid normalization and collapsed-fragment
warnings remain; these sampled routes do not certify every point on the platform.

`work/map-compile/watermill-collision-2GsrXz` preserves Leicester's complete
compiled geometry exactly. Both Leicester and Wychford pass native construction
and all fifteen control apply/reset checks (14.66 seconds). Wychford's existing
mill still has elevated approaches without supporting terrain, so its receiver,
two masks, jump and three entrances remain omitted. Compilation does not move it.

The reviewed recipe is `refinement/catalogs/leicester-watermill-collision.json`.
Publication backups live in the two-map artifact's `publication` directory.
Leicester/Wychford scene pins are refreshed; all ten saved scenes reopen with the
tested gameplay. A fresh published Leicester descriptor equals the staged result.
The 24 fresh published placement descriptors in
`work/map-compile/watermill-placement-mKTGrj` also equal their native-tested
counterparts. This publication does not refresh an existing baked mod ZIP.

## Native movement-obstacle mouse rejection (2026-10-05)

Native level loading registered ordinary movement obstacles with only the motion
flag, so mouse-sector queries could select the surrounding walkable area even
inside a building's movement obstacle. Obstacles now participate in mouse queries
as well. A native load regression verifies the obstacle interior, edge and vertex
are blocked while adjacent floor remains selectable. The movement suite passes
170 tests (ten ignored), level-loading checks pass 40 (three ignored), and all
50 fast-find-grid checks pass. The updated `robin` game binary builds successfully.

The explicit endpoint audit accepts optional `blocked_points`. Unlike boundary
intersection alone, these reject positions deep inside an obstacle. With the
runtime fix, the staged watermill cardinal-placement batch below passes all twelve
body-interior checks and 24 directed actor crossings. The published-definition
negative control in `work/map-compile/watermill-negative-vLBjYY` correctly fails
at the same body point. This separates missing asset collision from the runtime
selection bug; fixing one does not fix the other.

## Watermill collision audit: unpublished candidate (2026-10-05)

The published watermill has no movement blockers or selected movement solids.
Its original scene supplies exclusions, but placing the asset on new terrain
does not construct body collision. Its drop height is already correct: explicit
placement-ground metadata produces the identical inserted editor document.

The staged candidate derives collision from solid parts except the platform,
adds a platform clearance and selects the receiving region through the platform
entrance. `work/map-compile/watermill-placement-EIgfKI` compiles twelve placements
at three elevations and four cardinal rotations, retaining three entrances,
one jump pair and two masks. `work/map-compile/watermill-collision-tgsIKN` has
exactly the same complete Leicester geometry as the published edge-bank baseline.
Wychford still reports the elevated mill's missing receiver and approaches.

The expanded check in `work/map-compile/watermill-placement-bZjhGM` rejects the
45-degree placement: body collision blocks the raised platform entrance. The
clearance was projected to the lower plane before rotation; that construction
does not preserve the physical platform's projected footprint at other angles.
This candidate is not published. The native cardinal-placement checks above do
not establish arbitrary-rotation traversal or complete collision coverage.

## Published edge-bank receiver and mask attachments (2026-10-05)

`leicester-southwest-edge-bank` now binds its physical receiver and ground
occlusion mask within one authored bank relief (50.001003 units) above/below
their local anchors. The receiver previously had an eight-unit reach. The
reviewed recipe in `refinement/catalogs/leicester-bank-terrain-attachments.json`
explicitly pins those old bounds before replacing them. The authoring tool
rejects a missing or different old attachment, while reapplying the installed
recipe remains safe and repeatable. Six authoring tests, pipeline typecheck and
lint pass.

The staged descriptors in `work/map-compile/edge-bank-attachments-9n7wM3` preserve
Leicester's complete compiled geometry exactly. Wychford's navigation and physical
point arrays are unchanged; obstacle 402 gains receiving area `[167, 0]`, and two
bank masks are restored (17 to 19 masks). Its third mask anchor remains outside
the export frame and is correctly omitted. Twelve new placements at three terrain
offsets and four rotations bind both receiver and mask; four beyond the finite
reach reject. Native construction passes for both maps (13.83 seconds), all
fifteen controls pass apply/reset (11.37 seconds), and the focused receiver audit
passes 30 directed actor crossings over 15 affected receiver pairs (15.35 seconds).
This is uncalibrated Wychford geometry and a sampled receiver audit, not whole-scene
visual certification.

The definition is published, with transactional backups in the artifact's
`publication` directory. Leicester/Wychford pins are refreshed. All ten saved
scenes reopen, and their pinned bank gameplay equals the native-tested candidate.
The watermill is now the only remaining omitted physical receiver in this
Wychford descriptor. Other placement, traversal and visual gaps remain; no baked
ZIP was regenerated by this publication.

## Published footbridge deck ownership (2026-10-05)

The reviewed bridge and Leicester terrain definitions are now published through
the transactional gameplay installer. The review catalog is
`refinement/catalogs/leicester-footbridge-ownership.json`; backups and exact
edits are in `work/map-compile/footbridge-ownership-xRp193/publication` and its
parent directory. Models and placements are unchanged. Leicester and Wychford
scene references now use the updated descriptor hashes.

`work/map-compile/published-footbridge-lpfyyF` contains fresh exports from those
published definitions. All ten pinned saved scenes reopen. Leicester's complete
descriptor equals the staged, tested ownership migration. Both affected maps
pass native construction (14.01 seconds) and all fifteen compiled controls pass
apply/reset (13.93 seconds). Wychford has 23 movement areas and 17,478 sight
obstacles in this uncalibrated descriptor; Leicester has 55 areas and 444 sight
obstacles. These exports do not update the baked browser ZIP.

Wychford's missing footbridge receiver is resolved. Its remaining physical
receiver omissions are the southwest edge bank and watermill. The bridge ends
still warn about unmatched sockets because the current terrain does not meet
their heights. Leicester's upper socket also has no automatic match: its
existing controlled drawbridge passages remain the intended connection. Neither
warning establishes a missing deck or justifies bypassing those passage rules.

## Staged footbridge navigation ownership migration (2026-10-05)

The fixed bridge's upper neighbour is a stateful drawbridge. Its connection must
retain the existing controlled passages, rather than merging both navigation
regions. The lower end can join ordinary terrain through an explicit socket.

`work/map-compile/stage-footbridge-ownership.mjs` stages an asset-owned deck,
subtracts its projected footprint from Leicester's terrain boundary, preserves
the terrain's authored exclusion contours, and authors a matching lower socket.
The staged outputs in `work/map-compile/footbridge-ownership-xRp193` retain
Leicester's doors, controls and physical sight obstacles exactly. Moving the
bridge 300 units removes its old footprint from terrain navigation. The lower
connection in the equivalent `footbridge-ownership-WgfrPw` descriptor passes two
directed native actor crossings, and all twelve controls pass apply/reset. A
sampled ground-boundary audit found no eligible seam there; the explicit route
test supplies the actor evidence instead.

This exposed two assembly issues, now fixed: accepted sockets could remain
separated by a clipping crack below their matching tolerance, and preserved
terrain boundaries could not join ordinary authored floors. Preserved regions
now close cracks within 0.0001 map units while retaining larger separations.
Mixed boundary policies retain crossing exclusions and ordinary floor blockers,
independently of input order. Six focused assembly tests and 175 compiler/export
tests pass. The migration's exact bridge metadata also passes the new-placement
checks in `work/map-compile/footbridge-endpoints-00UfHo`, including all 30 native
actor routes and five raised-landing rejection cases.

The native endpoint audit now accepts an explicit layer and sector for routes
inside a larger map and uses the descriptor's actual bounds. Without these
indices it still requires a single region. The asset and terrain edits remain
staged, awaiting publication and saved-scene pin refresh. The drawbridge-side
socket remains unmatched by design in Leicester; its doors provide that route.

## Asset-owned footbridge deck and endpoint traversal (2026-10-05)

The unpublished east-village footbridge candidate now has explicit navigation
sockets on both short deck edges. `work/map-compile/check-footbridge-endpoints.mjs`
places this asset above two separately authored terrain landings at 0, 45, 90,
180 and 270 degrees. Each assembly compiles to one navigation region. Raising
both landings by 20 units instead leaves both endpoints unmatched in all five
cases; the compiler does not invent a connection across the height gap.

`work/map-compile/footbridge-endpoints-RVRKim` contains the descriptors, explicit
routes, candidate gameplay and rejected-landing checks. Native test
`exported_endpoint_routes_support_actor_crossings` passes 30 directed routes:
each endpoint and the complete landing-to-landing span, in both directions at
every rotation. It checks actor receiver and height updates throughout. The
test helper's finite tick allowance now scales with route length: the previous
200-tick limit expired during uninterrupted movement along the longer rotated
span. Runtime movement behavior is unchanged.

Reproduction uses `ROBIN_ASSET_MAP_DIAGNOSTICS` pointing at that artifact directory
with `cargo test -p robin_engine --lib exported_endpoint_routes_support_actor_crossings
-j1 -- --ignored --nocapture`. The endpoint audit requires explicit routes and a
single exported navigation region, rather than relying on sampled eligible seams.

The candidate remains unpublished. Staged original-scene exports in
`work/map-compile/footbridge-deck-Zak8FY` still report both endpoints unmatched.
Leicester's previous receiver shares navigation owned by the terrain asset;
the ownership migration must preserve traversability while making the deck
movable. Wychford also has a placement mismatch: the deck-end midpoints are at
66.000008 and 16.000005 units, while other receivers beneath their projected
positions are at approximately 4.246223 and 7.829813. Merely adding sockets cannot
make those landings meet. These measurements come from the uncalibrated staged
descriptor and do not certify the surrounding routes.

## Sloped asset sockets meeting authored terrain (2026-10-05)

Exterior navigation sockets now compare the receiving terrain plane at both
socket endpoints. Previously the compiler compared the socket's midpoint height
with a probe offset into neighboring terrain, rejecting a continuous slope.
Matching just the midpoint could also accept a differently tilted surface. The
offset probe still selects adjacent unblocked terrain; it no longer supplies the
height comparison. Coplanar terrain subtraction now uses fixed-point clipping to
avoid degenerate floating-point fragments at rotated shared edges.

The compiler regression covers matching and mismatching slopes at 0, 45, 90,
180 and 270 degrees, translated and elevated. All 112 gameplay compiler tests
and 63 map-export tests pass. The export test compares complete descriptors and
routes against `asset-sloped-terrain-sockets.json`. Native test
`actors_cross_sloped_terrain_sockets_after_rotation` walks an actor across the
actual deck/terrain connection in both directions at all five rotations, checking
receiver and height updates throughout. All ten directed routes pass. This
explicit test supplements the sampled seam audit, which found no eligible deck
crossing at 45 degrees and therefore could not prove that case on its own.

This fixes connections in new assemblies; it does not publish the footbridge
candidate or resolve its existing-map navigation ownership.

## Footbridge deck ownership investigation (2026-10-05)

The published `leicester-east-village-footbridge` has no walkable surfaces. Its
physical receiving volume has a sloping top matching the modeled deck, but its
receiver anchor depends on another asset's navigation. This fails in Wychford.

The unpublished experiment `work/map-compile/stage-footbridge-deck.mjs` assigns
that top to an asset-owned walkable surface using `projectionVolume`. Outputs in
`work/map-compile/footbridge-deck-UyLvKh` remove the east-footbridge receiver
warning and pass native construction for both Leicester and Wychford (9.06
seconds). This is **not a usable fix**: Leicester gains an independent deck layer
(18), and the focused actor receiver audit for obstacle 385 finds zero eligible
crossings and fails its nonempty-coverage assertion. The baseline receiver instead
belongs to the larger navigation region on layer 1. Native construction alone
would miss this connectivity regression.

Do not publish this candidate. The next work is to author the deck's endpoint
connections and resolve navigation ownership with its surrounding surfaces,
then verify traversal in the existing scenes and in newly placed assemblies.
The staged Wychford descriptor does not include spline mesh calibration.

## Published woodland-bank terrain attachment (2026-10-05)

`leicester-bank-terrain-attachments.json` now authors a finite vertical probe for
`leicester-southwest-woodland-bank`'s ground receiving volume. Its reach is one
authored bank relief (50.001003 units) above/below the base, derived from the
physical shape rather than a scene-specific terrain height. The asset definition
and Leicester/Wychford pins are published; all ten saved scenes reopen.

The staged outputs in `work/map-compile/woodland-bank-attachment-BYht4j` preserve
Leicester's complete compiled geometry exactly. Wychford gains receiver bindings
for sight obstacles 404 and 405. Navigation geometry and physical points are
unchanged; virtual building references are renumbered to account for the two new
receivers. Both maps pass native construction (6.08 seconds) and all 15 controls
pass apply/reset (9.27 seconds). A focused Wychford actor audit passes 74 directed
crossings across 37 eligible receiver pairs touching those two obstacles. The
report explicitly records its map and receiver filters; it is not a full-map walk
audit. Twelve new placements at three relative terrain heights and four rotations
bind, while four placements beyond the finite reach reject the receiver.

The staged Wychford export did not prepare spline mesh calibration. The remaining
physical-receiver omissions concern the footbridge, edge bank and watermill;
the church traversal and elevated tower entrance remain separate gaps. The latest
baked ZIP and prior calibrated descriptor predate this metadata publication.

## Refreshed published-map batch (2026-10-05)

`work/map-compile/published-probes-20261005-r2` contains fresh best-effort exports
of all ten saved scenes using their pinned asset definitions. Native construction
passes for every descriptor (8.66 seconds), and all 71 compiled controls pass
apply/reset (7.60 seconds). Controls per map are Wychford 3, Croisement01 6,
Croisement02 9, Croisement03 9, Derby 5, Leicester 12, Lincoln 11, Nottingham 10,
Sherwood 0 and York 6. Wychford now includes the published west-tower reveal.

This batch does not prepare mesh calibration for Wychford's spline wall; its
22 areas and 17,478 sight obstacles are an incomplete wall export, not a replacement
for the separately calibrated run. Wychford retains the church approach-height
mismatch and missing/ambiguous physical receivers on the footbridge, edge banks,
woodland banks and watermill. Five Lincoln props still have no gameplay definitions:
the barn tool, loose log, path poles, loose plank and south stake. Their published
parts are marked scenery and contain no authored collision shapes; this audit
does not assume that they should be non-colliding. Integer-grid collapses and
recovery-review warnings also remain. Successful loading and state changes do not
certify omitted geometry, rendered output or complete actor traversal.

The fresh calibrated Wychford run is
`work/map-compile/wychford-wall-calibration-UNeRFM`. Model calibration reports no
warnings. Its descriptor has 146 movement areas, 21,323 sight obstacles, 30 doors,
17 masks, two lifts and three controls. Native construction passes (5.97 seconds),
including 57,456 grid blocks and 36,983 elevation boundaries; all three controls
pass apply/reset (5.72 seconds). The church mismatch and five physical-receiver
binding omissions listed above remain after calibration. This is a descriptor
audit, not a newly baked/rendered ZIP or an actor-route certification.

## Disconnected spline-light receiving probes (2026-10-05)

Spline lighting now retains each disconnected fragment of a clipped receiving
probe. The existing light receiver compiler resolves each fragment independently;
no segment is invented across a trimmed gap. Straight, partially trimmed and
curved fixture paths preserve the same light regions as their point-anchored
counterparts, without mutating the asset. The complete split-probe descriptor
matches `asset-spline-material.level.json`; the native lighting regression passes
region/layer and ambience-filter queries for all three repeated sections.
All 19 focused spline/editor tests, app typechecking and targeted lint pass.
This does not remove omissions for probes with no surviving receiver or ambiguous
intersections, nor verify complete-scene lighting and shadows.

## Disconnected spline-mask receiving probes (2026-10-05)

Mask authoring now supports disconnected `receiverPolylines`, mutually exclusive
with the existing single segment/polyline forms. Spline clipping retains all
surviving fragments instead of dropping the mask. Each fragment is transformed
and intersected independently; the compiler still requires one receiving layer.
Tests reject an invented connection across a gap, competing layers, empty or
degenerate fragments and conflicting probe forms. Curved and partially trimmed
walls retain the fragments without altering the input asset.

The split-probe export exactly matches `asset-spline-material.level.json`, and
the native mask regression passes bitmap coverage, character/projectile boundary,
altitude and independent obstacle ownership checks. Disconnected application
boundaries remain unsupported; this change resolves receiving probes only.
All 129 focused editor/compiler tests pass, along with app/pipeline typechecking
and targeted lint.

## Changing climb barriers and corrected fixture diagnosis (2026-10-05)

Follow-up verification covers copied controls and closing a barrier during a
climb. Three exported assemblies each contain two copies of a ladder, ordinary
wall or crenellated wall. Closing each copy and then resetting them independently
passes 48 directed actor/state checks. A separate per-tick test applies the
barrier only after the actor has climbing posture and occupies the lift sector,
asserts that the active barrier blocks the segment ahead, and requires traversal
to stall. All twelve placed fixtures pass in both directions (24 further checks).
The copied stair regression still passes; 62 editor export tests, app typechecking
and targeted lint pass. These tests use complete animation and current compiled
asset definitions; no gameplay-engine change was needed.

Export now retains changing ladder/wall barriers. Twelve compiler-generated
fixtures cover ladders and both wall-top door types at four rotations. Native
collision queries and pathfinding reject the applied barrier in both directions;
complete character animation stalls while closed and traverses before apply and
after reset. The placed fixtures pass 72 directed actor/state checks. The route
test runs without external resources; the animation test requires `ROBIN_CLIMB_RHS`.
Editor tests compare the native fixtures with fresh compiler output.

The earlier reported rotated-wall runtime failure was an invalid fixture
expectation: changing a stair's type to wall also adapts the high approach by
60 units, leaving both endpoints on one side of that fixture's barrier. It did
not prove that motion crossed a blocking line. Adding collision and pathfinder
assertions exposed that mistake. The new asset fixture provides enough space
for actor clearance and the fixed 60/65-unit wall approach.

An additional narrow barrier at local X=93..95 tests complete entrance/exit
animations in the small fixture. Although clearance adjustment moves the inner
endpoint beyond this barrier, the complete actor route still stalls while closed.
Both ladder and wall pass in both directions before apply, while closed and after
reset (12 further checks, 84 total). An inner-endpoint reachability query alone
would incorrectly diagnose this case as a bypass. All three native tests pass;
171 focused editor tests, app typechecking and targeted lint also pass.

Broader placement coverage, reopening during an existing movement sequence and
rendered state transitions remain unverified. These results are not full
climbing parity.

## Published church insertion ground elevation (2026-10-05)

The church-side tower now publishes `placementGroundHeight: 50.0010129354411`,
the local elevation of its lowest external stair approach. New drops align that
approach with the requested ground elevation while preserving the stair and
foundation geometry. Twelve actual insertion checks cover ground heights 0,
40 and 130 at four rotations. Leicester's compiled geometry before and after
the metadata change is exactly identical.

The reviewed value is recorded in
`refinement/catalogs/leicester-placement-ground.json`; validation and publication
snapshots are under `work/map-compile/church-placement-ground-lnFlbN`. The
Leicester and Wychford descriptor pins are refreshed. Existing placements are
unchanged, so Wychford's already-elevated church approach still needs supporting
terrain or an intentional placement edit. These insertion checks do not prove
that independently placed upper landings connect.

## Authored placement ground elevation (2026-10-05)

Investigating the church traversal confirmed a physical landing mismatch: its
lower approach is authored around local height 50, while its geometry extends
down to the local base. Snapping an endpoint down to terrain would not extend
the stairs. New asset insertion now accepts optional
`gameplay.placementGroundHeight` to place a reviewed ground/entrance elevation
on the requested terrain, leaving foundations below it. Without the field,
the existing lowest-surface/collision placement rule remains in use.

Insertion, save/reopen and invalid-value checks pass among thirteen asset-command
tests; app typechecking and targeted lint pass. This mechanism does not move
existing placements or change compiled endpoints. The church asset's specific
placement height has not yet been published, and its existing Wychford placement
still lacks the required lower landing.

## Published west-tower reveal terrain attachment (2026-10-05)

The west tower's appearance/mask-only reveal now owns a finite receiver probe
from one unit below the modeled base to one unit above its existing raised
approach. Derby's entire compiled geometry remains exactly identical. In
Wychford, this restores the reveal and its two applied masks: three controls
and seventeen masks instead of two and fifteen. Navigation, sight obstacles,
doors and traversals remain unchanged. The elevated entrance is still omitted
when no supporting floor exists; the probe does not relocate that entrance.

Both descriptor fixtures pass native construction and apply/reset of all five
Derby and three Wychford controls (5.35/5.74 seconds). Evidence and publication
snapshots are in `work/map-compile/tower-control-attachment-2NjWaI`; the reviewed
recipe is `refinement/catalogs/derby-control-terrain-attachments.json`. The asset
definition and referencing scene pins are published. This focused Wychford
batch omits wall calibration; the previously baked ZIP predates this fix and
must not be described as validating the newly restored control.

## Reviewed terrain attachment for controls (2026-10-05)

The terrain-attachment authoring utility now supports `control` recipes targeting
an asset-local transition waypoint. Rules retain explicit owner, anchor and
finite vertical reach checks; existing waypoint anchors or conflicting receiver
segments are rejected. Runtime compilation still requires exactly one receiving
surface. Tests cover sloped ground, competing stacked floors, repeatable
authoring and unchanged input definitions. Fourteen focused tests, pipeline
typechecking and targeted authoring lint pass.

This adds the missing authoring path for controls such as Wychford's displaced
tower reveal. No tower probe bounds have been published yet: choosing and
validating those bounds against the tower's intended approach remains separate
from providing the authoring mechanism.

## Calibrated Wychford browser ZIP round trip (2026-10-05)

The corrected full-library browser fixture produces
`work/map-compile/wychford-wall-calibration-QcegIB/editor-wychford-current.zip`
(51,927,200 bytes, SHA-256
`0c76ca3c7735ce08709a4077480b7581236d27cb4d1f1d75eee8d1fa8fc2b2e7`).
The 3600×2400 baked map was visually inspected. The native
`full_editor_archive_constructs_native_map_without_base_datadir` test passes
in 6.67 seconds: color/depth/minimap and the embedded editor scene load, with
21,323 sight/receiving obstacles, 15 masks, 30 door projections, 57,456 grid
blocks and one appearance region for two controls. The descriptor contains
two traversals and no animated scenery.

This verifies the real packaged resources and native archive path after wall
calibration. It does not certify rendered actor occlusion, every traversal,
state interaction or the still-omitted map connections. The existing user mod
archive was not overwritten.

## Fresh calibrated Wychford construction (2026-10-05)

The current scene with model-calibrated walls exports to
`work/map-compile/wychford-wall-calibration-QcegIB`. Native construction passes
with 146 areas, 21,323 sight/receiving obstacles, 30 doors, 57,456 grid blocks and
36,983 elevation boundaries (8.69 seconds). Both compiled controls apply/reset
(8.45 seconds). The church-side traversal endpoint remains at height 134.10
above a receiving floor at 88.33; the west-tower control remains at 196.70 above
a floor at 88.21. These are unresolved placement/attachment omissions.

The full-library browser bake fixture was also corrected to forward the
calibrated descriptors supplied by the viewport to its export worker. It had
captured the uncalibrated input map instead, so previous browser checks using
that fixture cannot certify spline-wall gameplay. App typechecking passes;
this correction has not yet been verified by a new rendered ZIP round trip.

## Fresh ten-map native construction and controls (2026-10-05)

After repairing Lincoln's spire bindings, the completed descriptor batch at
`work/map-compile/published-mask-probes-20261005` passes native geometry
construction for all ten maps (9.09 seconds). All 70 compiled controls apply and
reset (9.04 seconds): Croisement01/02/03 have 6/9/9, Derby 5, Leicester 12,
Lincoln 11, Nottingham 10, Sherwood 0, Wychford 2 and York 6.

This checks best-effort descriptor loading and compiled controls, not complete
feature coverage or rendered ZIP round trips. Five recently added Lincoln props
lack gameplay definitions. Wychford in this generic batch lacks model-derived
wall calibration and omits one traversal, one control, fifteen masks and
twenty-five doors. A separate calibrated-wall audit is required; these omissions
must not be mistaken for a full-map pass.

## Lincoln spire appearance binding repair (2026-10-05)

A fresh published-map audit stopped at Lincoln: the spire model now exposes
two appearances, but its gameplay still mapped its only control to the former
local appearance number. The shared preview name consequently collided with
the hall's other control. The spire's existing upper reveal now binds to
`appearance-2`; a separate `appearance-1` control connects to the hall's lower
reveal through an asset-local spatial join. The compiler's ambiguity check is
retained.

Both definitions are published and Lincoln's descriptor pins are refreshed.
The baseline constructs natively and all eleven controls apply/reset. A joint
one-unit hall/spire move also constructs and applies/resets its ten surviving
controls, but exposes unresolved neighboring receiving geometry: one control,
a traversal, doors and light bindings are omitted. The moved result is not a
placement-parity pass. Staged descriptors, warnings and publication snapshots
are under `work/map-compile/lincoln-spire-repair-yblqgl`.

The preceding all-map batch at `work/map-compile/published-mask-probes-20261005`
compiled six scenes before the Lincoln failure; its incomplete output is not
an all-map acceptance result. The remaining maps and calibrated Wychford still
need fresh verification.

## Cropped mask anchors with surviving probes (2026-10-05)

A cropped wall repeat now retains its mask when the authored point anchor is
trimmed but an explicit receiving probe survives. The generated anchor moves
to that surviving probe for export-frame checks; layer selection still requires
the probe to intersect exactly one receiving layer. No nearest-surface fallback
is introduced. A three-repeat regression retains the shortened final mask;
removing the probe correctly restores the cropped-anchor warning and omission.
Point-only cropped anchors and disconnected probes/boundaries remain open.
The complete editor suite passes 745 tests with two skipped (44.6 seconds);
app typechecking and targeted lint also pass.

## Bend-preserving mask receiver probes (2026-10-05)

Mask definitions now accept a finite `receiverPolyline` instead of a straight
`receiverSegment`. Spline compilation clips and subdivides authored probes at
deformation stations, preserving their bends. All intersections must select one
receiving layer; repeated hits on that layer are allowed, while missing or
competing layers still fail strict export and warn/omit in best-effort export.

The curved-wall regression now exports longitudinal probes. A cropped repeat
whose surviving probe misses the ground correctly warns while other repeats
export. Ordinary asset tests cover repeated intersections, ambiguous stacked
surfaces, missing intersections and degenerate probes. The shared native mask
fixture now originates from longitudinal probes and retains identical exported
bytes, including mask layers, bitmap data and obstacle links. This supersedes
the longitudinal-segment limitation below; cropped anchors and disconnected
probes/boundaries remain incomplete.

## Spline mask coverage and ownership (2026-10-05)

Wall masks now deform and crop their coverage triangles, character/projectile
boundaries and receiving anchors with each wall repeat. Obstacle references bind
only to the corresponding repeat's generated volume fragments. Open boundaries
retain their authored meaning; closed boundaries rebuild their front envelope
after placement. Degenerate projected triangles are omitted.

The editor regression covers a cropped final repeat and curved paths, unchanged
source definitions, and preserved collision when an unsupported mask is omitted.
The shared export fixture passes native bitmap checks, character and projectile
boundary queries, projectile top-plane and flying-human bottom-plane queries,
and isolation between all three repeated sections. All eight native spline
integration tests pass; the 16 focused editor tests, typechecking and targeted
lint pass.

This is partial mask support: cropped receiving anchors, longitudinal receiving
segments and disconnected cropped boundaries still warn and omit affected masks.
Full-scene rendering, changing wall sources and elevated/stacked mask receiver
coverage remain unverified. These checks do not establish full visual parity.

## Automatic curved-wall lighting attachment (2026-10-05)

Unanchored spline lights now find matching asset-local receiving planes before
deformation. Their generated fragments derive finite probes from overlaps with
those same deformed surfaces, including the exported contour's quantization.
This avoids requiring independently tessellated light and walking triangles to
have identical planes. Source ownership stays local to each wall span/repeat;
missing overlaps warn and omit only the affected light fragment.

The previously failing curved/rising case now matches explicit-probe output.
All 16 focused editor tests pass, including clipped overlap, reversed winding,
fractional origins and golden export equality. Typechecking and targeted lint
pass. The new golden fixture also passes native shadow queries for ambience
masks 1, 2 and 4: the wall-top layer receives shadow while the terrain underneath
and points outside the light remain unaffected. This closes the specific
automatic plane-matching gap described below, not every lighting/visual case.

## Spline lighting receiving segments (2026-10-05)

Authored receiving segments now clip and subdivide at the wall's deformation
stations. Generated `receiverPolylines` retain those bends. The compiler resolves
all segments of each probe together and requires one receiving sector, retaining
the existing rejection of stacked or absent receivers. Disconnected crops warn
and omit the affected probe rather than creating a false connecting segment.

Focused tests cover rising and curved wall tops, multi-segment probes whose
individual segments miss the surface, stacked-floor rejection and malformed
polylines. The full editor suite passes 740 tests with two skipped (45.27 seconds).
The golden fixture now includes both a point and segment probe and
still matches the descriptor validated by native lighting queries. Typechecking
and targeted lint pass.

An automatic-binding gap was reproduced by removing the explicit
probe from the curved/rising wall case in `wall-spline-gameplay.test.ts`: exact
plane matching finds no receiver for its first light fragment. Explicit probes
worked; source-surface attachment described above subsequently fixed the
unanchored case without requiring equality of independently tessellated planes.

## Spline lighting point anchors (2026-10-05)

Explicit point anchors now deform with each repeated light region. Subdivided
contours share an asset-local receiver-coverage group, namespaced per placement
and repeat; an anchor may select the receiving layer for the entire group but
cannot borrow coverage from an unrelated light. Generated points use vertical
probes of ±1/1024 game unit to accommodate deformation/receiving-plane rounding.
Wrong-height anchors still fail. A crop removing every anchor omits that repeat's
region with a warning. Authored receiving segments remain unsupported on walls.

All 131 focused compiler/draft/wall tests pass, including rising wall tops,
invalid groups, unrelated coverage and wrong-height rejection. The seven app
wall tests pass: explicit anchors produce the same golden descriptor already
validated by native shadow/layer/ambience queries. Typechecking and targeted
lint pass. This is not complete attachment coverage for every slope or asset.

## Spline-wall spatial sounds (2026-10-04)

Point and polyline emitters now follow repeated wall sections, source trimming,
corners and path deformation. Polyline segments gain the shared wall sampling
stations before deformation so curved paths retain their intermediate shape.
Acoustic distances, volumes, altitude category, timing and ambience masks remain
asset-owned. Cropping never connects disconnected surviving fragments; those
cases currently omit the emitter with a warning. Global emitters likewise warn
instead of being multiplied along the wall.

The full editor suite passes 738 tests with two skipped (52.76 seconds), including
reverse-direction clipping, bends, disconnected fragments, point emitters,
partial repeats and curved paths. The golden export passes native construction
with three independently placed emitters, one required sound sample, retained
delay settings and converted runtime volume. All seven native spline integration
tests pass. This validates construction and placement, not audible playback.

## Spline-wall same-plane lighting (2026-10-04)

Wall-local light/shadow contours now follow repeated source spans and path
deformation, retaining their ambience masks. Receiving layers are rebuilt from
the deformed planes. Compiler tests cover trimmed repeats, turns, a rising
wall-top surface and explicit omission warnings for receiver anchors that are
not yet supported. The focused editor wall suite passes 13 tests; typechecking
and targeted lint pass.

The golden export fixture passes native shadow queries on all three repeated
sections, distinguishing lit and unlit points, receiving layers and ambience
masks 1, 2 and 4. All six native spline integration tests pass. Explicit point
and segment receiver anchors remain unsupported on spline walls; their regions
are omitted with a specific warning. No mission ambience selection is added to
the map.

## Terrain-junction receiver correction (2026-10-04)

Crossing all incident elevation edges can cycle back to a face the actor has
left. Landing exactly on their common vertex also suppresses those edges on
the next step, leaving a stale receiver on departure. The runtime correction
resolves the destination when a multi-edge dispatch leaves the actor outside
its receiver, and handles exact-boundary departure through the usual movement
update path. Ordinary directional ownership at shared boundaries is retained.

The synthetic sloping four-triangle fan passes in both axes and directions;
the audit still rejects stale interior receivers and unequal-height vertices.
All 48 directed crossings in the reduced Wychford fixture pass in 0.13 seconds.
The broader movement suite passes 182 tests with five ignored in 9.62 seconds.
The complete-animation climb regression also passes after the correction:
2,166 directed routes over ladders and both climbable wall types (15.69 seconds).
The updated game build passes (`cargo build -p robin_rs --bin robin -j1`, 85
seconds). The full Wychford receiving-seam audit subsequently passes 17,468
directed actor crossings over 8,734 eligible pairs in 1,054.35 seconds. Its
`actor-receiver-crossing-report.json` in the calibrated Wychford diagnostic
directory records `complete: true`. This samples initial-state receiver seams;
ground boundaries, other control states and other feature categories remain
separate acceptance checks.

## Spline-wall material regions (2026-10-04)

Wall compilation now deforms asset-local material contours and rebuilds their
ground, obstacle and receiving-surface ownership for each repeated source span
and corner. Vertical faces triangulate in their dominant plane. Trimming and
integer-grid collapse omit unusable fragments with explicit collapse warnings;
quantization uses the export image origin. Material IDs remain unique when
trimming removes owners between wall spans. Source definitions stay unchanged.

The focused wall/export suite passes 11 tests, including moved/trimmed repeats,
turned and sloped paths, corners, ground registration, vertical faces, receiver
ownership and fractional image origins. Editor typechecking and targeted lint
pass. The full editor suite passes 734 tests with two skipped (91.64 seconds).
An editor-export golden fixture also passes native ground and obstacle material
queries across three repeated wall sections. Each section retains its own
regions, uncovered points use the appropriate default, and ground-only regions
do not override obstacle materials. These checks do not establish complete wall
traversal, receiver-material coverage or visual parity.

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

### Nottingham southwest wall stair publication

Candidate `nottingham-southwest-wall-stair-contact-zlABhr` corrects independently
owned stair, wall and terrain contacts. Receiver-footprint corrections alone
left twelve full-map routes failing; a clearance owned by the wall and matching
its corrected walkway restores support. `nottingham-southwest-wall-clearance-F2RhJc`
passes 92 actor stair routes and ten control apply/reset checks.

`nottingham-southwest-wall-stair-neighbour-placements-E8wjXz` passes 192 routes
across two copies, four rotations and two elevations; 32 missing/raised wall
cases reject. Fixtures contain only selected assemblies and authored terrain.
Publication backup: `nottingham-southwest-wall-publication-20261006`. Fresh
published exports match the full-map candidate and all eight placement exports;
all ten scenes reopen. Audit `lift-anchor-support-YdJEnk` finds 18/53 unsupported
stairs, 1/8 ladders and 9/9 walls.

Flight mesh support is 833/833 samples and terrain contact support is 205/205.
Upper wall contact support is only 15/205, with discrepancies up to 32.484008
game units. The draft warning remains; mesh repair and rendered verification
are still required.

The stricter upper-castle fixture
`nottingham-castle-upper-stair-neighbour-placements-UBfIdR`, excluding background
terrain, also passes 352 routes and 96 missing/raised-neighbour cases.

### Nottingham north-wall stair publication

`upper-castle-stair-contacts-LhyR80` corrects the north stair and independently
owned north curtain landing. `nottingham-north-wall-connected-LPAX0h` passes all
92 stair routes and ten control apply/reset checks.
`nottingham-north-wall-stair-neighbour-placements-yHklUd` passes 672 routes across
two copies, four rotations and two elevations, with 32 missing/raised wall cases
rejected. The north fixture uses larger bounds so a rotated copy remains inside
the export frame; no production placement or connection tolerance is changed.

Flight support is 1,055/1,055 mesh samples. The wall contact has 184/205 hits
and a maximum discrepancy of 0.108970 game units, recorded as a draft warning.
Publication backup: `nottingham-north-wall-publication-20261006`. Fresh exports
match all sixteen north/southwest placement descriptors and the combined map
candidate `nottingham-both-wall-stairs-YKN43y`; all ten scenes reopen.
The combined candidate passes all 92 actor stair routes and ten controls.
Audit `lift-anchor-support-TFhGth` reports 17/53 unsupported stairs, 1/8 ladders
and 9/9 walls. Nottingham now has zero unsupported stair definitions locally;
this is not a claim of complete placement or rendered parity.

### York courtyard lodge stair publication

Stair-only candidate `local-stair-seams-ZT42ON` passes 32 synthetic external
landing routes and 64 missing/raised cases in `external-stair-landings-WbGN5B`.
Full-map candidate `york-lodge-stair-2LRNp0` fails both routes through sector 187
at its upper wall contact. Correcting the independently owned rear curtain wall
in `upper-castle-stair-contacts-HCToJE` restores all 130 York stair routes and six
control apply/reset checks in `york-lodge-connected-qomrpy`.

`york-castle-courtyard-lodge-stairs-neighbour-placements-rQBxu5` tests the actual
raised-terrain and wall neighbours in two copies at four rotations and two
elevations: 32 actor routes pass and 64 missing/raised neighbours reject. Fresh
published exports match the full-map candidate and all eight placement files;
all ten scenes reopen. Publication backup: `york-lodge-stair-publication-20261006`.
Audit `lift-anchor-support-KRS00w` finds 16/53 unsupported stairs (York 15/16,
Sherwood 1/1), 1/8 ladders and 9/9 walls.

Mesh review finds 685/693 flight hits with a maximum gap of 0.225310 units, and
164/205 wall contact hits with a maximum gap of 0.579569 units. These limits are
explicit authoring review bounds, not runtime connection tolerances. Draft
warnings preserve these discrepancies and the requirement for rendered review.

### York east riverside stair publication

Stair-only candidate `local-stair-seams-OWJLOR` passes 32 copied routes and 64
missing/raised synthetic landing cases (`external-stair-landings-tGjuCK`), but
full-map `york-riverside-stair-7AG6Mq` fails both routes at the actual contacts.
The lower contact is authored in the terrain's projected frame beneath a
90.00101-unit receiver. The upper contact belongs to the south water-gate bastion.
Moving its edge initially overlaps the adjoining wall; intersecting the corner
with the shared wall boundary and preserving that wall's movement precision
fixes both original and rotated compilation without relaxing material checks.

Final candidate `york-east-riverside-curtain-wall-contact-2wGhpx` passes all 130
York stair routes and six controls in `york-riverside-precision-gzZs5u`.
`york-east-riverside-curtain-wall-neighbour-placements-RKPcZl` passes 32 routes
over authored raised terrain, with independent wall/bastion copies at four
rotations and two elevations. Expanded fixture `...-0zEzOk` retains identical
exported geometry and rejects all 48 missing/raised bastion or terrain cases.
Fresh published geometry matches the full map and eight placement files;
all ten scenes reopen. Backup: `york-riverside-stair-publication-20261006`.

Flight support is 669/676 mesh samples (maximum edge gap 0.278843 units), upper
contact discrepancy stays below 0.135 units, and terrain support is 205/205.
These are draft mesh limitations, not runtime connection tolerances or rendered
verification. Audit `lift-anchor-support-zwj2QO` finds 15/53 unsupported stairs
(York 14/16, Sherwood 1/1), 1/8 ladders and 9/9 walls.

### Southern riverside stair: unpublished binding failure

`local-stair-seams-SbKFtN` corrects the southern riverside stair's local seams.
Mesh review has 719/723 flight hits with an edge discrepancy of 0.119457 units;
reviewed upper landing strips have complete sampled support. The first isolated
export omits both copies because its lower approach is blocked. Diagnostic
`external-stair-landings-8g6yrC` retains the failed scene and compiled descriptor.

Candidate `raised-stair-approach-IahH7Z` reprojects existing ground openings onto
the 50.001003-unit approach, restricted to the contact strip and owning wall
parts. Projected mesh-triangle intersections above the landing find zero overlap
for both 519.787-square-unit strips. All eight placements then compile and 32
missing/raised landing cases reject, but all 32 native routes fail.
Adding matching flight clearances (`raised-stair-approach-6OxQKK`) and updating
receiver footprints (`raised-stair-approach-3l1bgp`) does not fix traversal.
The latter's `external-stair-landings-EXkStY` reports that neither landing receiver
reaches its physical door. Preserved-boundary fixture
`external-stair-landings-AQmZsu` also fails all 32 routes. These are diagnostic
candidates only; none is published and the library audit count is unchanged.

The binding failure was traced to collision-clipped landing polygons, rather
than an engine tolerance issue. Matching flight/upper-floor clearances across
the three owning parts fixes the upper landing; reprojected ground openings
still leave a lower seam gap. Mesh-derived approach strips subtract projected
mesh above the landing plus a 0.1-unit authoring margin, preserving the solid
columns while clearing only the broad hull's empty space.
`raised-stair-approach-hcQSLF` passes all 32 synthetic routes in
`external-stair-landings-hanD6G`. Retaining the upper receiver's existing material
footprint avoids a full-map gatehouse overlap; candidate
`raised-stair-approach-IkVezD` still passes all 32 synthetic routes and 32
missing/raised landing rejections (`external-stair-landings-PjIi9R`).

Terrain-contact candidate `york-east-riverside-southern-wall-stair-contact-NZg1Ho`
has 205/205 supported terrain strip samples. Full-map
`york-south-connected-KHOKIZ` passes six controls but still fails two of 130 stair
routes. Its lower physical receiver belongs to
`york-southeast-riverside-raised-terrace` (`building-093`); the receiver edge stops
short of the corrected doorway even though the terrain boundary now matches.
Independent authored-terrain fixture
`york-east-riverside-southern-wall-stair-neighbour-placements-mjj6wU` passes 16/32
routes, with upper-landing failures at 37 and 90 degrees; all sixteen
missing/raised terrain cases reject. These receiver contacts remain unresolved
and none of the southern riverside candidates is published.

Further receiver checks distinguish two problems. Candidate
`york-terrace-receiver-SBTlub` corrects the terrace receiver edge with a maximum
mesh discrepancy of 0.149703 units (62/205 exact hits). In
`york-terrace-connected-YbERSp` both landing bindings now succeed, but the two
southern-stair routes still fail: a ground-plane collision contour remains across
the lower doorway. Engine binding errors now report each patch's distances to
the middle/outside anchors; all nine landing-binding unit tests pass.

The physical-floor alternative `york-terrace-receiver-KnFwcm` gives the terrace
its own height-50 walking region and volume-owned collision, without editing
the background terrain. `york-physical-terrace-00P595` restores the southern stair
but omits the riverbank wooden stair's lower entrance. Its 128 passing routes
are therefore a regression from 130, not a passing full-map result. The wooden
stair/terrace contact needs review before this alternative can be published.

Upper receiver extension alone (`raised-stair-approach-Wdtowj`, placement fixture
`york-east-riverside-southern-wall-stair-neighbour-placements-1fjsJs`) retains
16/32 passing routes. Failed bindings measure gaps of about 0.016 and 0.027 units
at 37/90 degrees; their exported landing boundaries have lost subpixel contours.
Preserving an explicit upper navigation region (`raised-stair-approach-RDo7AX`,
fixture `york-east-riverside-southern-wall-stair-neighbour-placements-DCfCkk`)
passes the first four routes, then native loading rejects the 37-degree export:
`invalid rounded precise motion obstacle: SelfIntersection(Exterior)`.
All candidates remain unpublished. The rounded-obstacle defect and the separate
wooden-stair contact are the next validation targets.

### Rounded landing corner repair

The rejected contour is an outer landing boundary, despite the shared validator's
obstacle wording. Rounding a valid precise boundary creates a crossed narrow
corner. The compiler and native validator now remove that corner only within
the grid-rounding width bound, retaining the exact contour. Larger crossings
remain invalid; changing the integer footprint independently still rejects.
A shared captured fixture covers both implementations.

Regenerated candidate `york-east-riverside-southern-wall-stair-neighbour-placements-ZHyCXL`
passes all 32 directed native actor routes at rotations 0/37/90/180 and elevations
0/40. Its report is finished with no skipped permissions or failures. This is
initial-state traversal, not complete-sprite or rendered verification. The
fixture also rejects sixteen missing/raised terrain cases. The separate full-map
terrace/wooden-stair conflict remains unresolved; no asset publication is implied.

Validation: 186 affected compiler tests, 86 level-data tests (eight ignored),
app typecheck, focused lint and Rust formatting pass. Pipeline typecheck remains
blocked by existing errors in `compile-asset-gameplay.test.ts` (optional assertion
message) and `state-delivery.test.ts` (endpoint union indexing and assertion import).

### Raised riverside walkway and actual terrace placements

The wooden stair's lower receiver belongs to `york-east-riverside-wooden-walkway`.
Its height-15 landing lies outside the height-50 terrace in world space, but
ground-plane navigation places terrace collision across its projected approach.
Candidate tool `stage-york-riverside-walkway-floor.mjs` gives the walkway an
asset-local physical floor and movement solid, retaining its receiver volume
and material associations. It combines the physical terrace candidate with
the corrected southern stair; it does not change the published library.

`york-riverside-walkway-floor-r9Co7e` restores all 130 full-York directed stair
routes and all six control apply/reset checks. The wooden stair still uses
projected navigation. This is a passing original-placement candidate, not
arbitrary-placement or mesh certification.

`check-courtyard-west-stair-neighbours.mjs --physical-terrace` tests the actual
terrace with independent stair/terrace copies and no background terrain.
`york-east-riverside-southern-wall-stair-neighbour-placements-i59PB6` compiles
eight placements and rejects 32 missing/raised terrace cases. Native traversal
finishes with 24/32 passing routes: all eight 37-degree routes stall at the
lower doorway. This remaining contact defect prevents publication.

Separate wooden-stair seam candidate `local-stair-seams-MSP8Tb` needs a
2.625315-unit floor-vertex adjustment. Its combined full-map candidate
`york-riverside-walkway-floor-teBjYS` emits physical navigation but fails the two
wooden-stair routes (128/130 pass). Mesh review finds only 450/862 floor samples
supported and an uncovered-edge distance of 13.657964 units. That floor requires
geometry review, not a larger runtime tolerance. All these candidates remain
unpublished.

### Exact collision partitions at rotated terrace contacts

The actual terrace's 37-degree failures came from holed collision being
triangulated after rounding. Its fourteen grid triangles had no matching exact
contours, so a rounded edge blocked the stair's lower entrance. Reconstructing
individual triangle edges afterward was insufficient: unrelated corners could
prevent a complete match, and partial recovery left this entrance blocked.

The compiler now partitions precise holed collision before rounding. Triangles
that collapse on the grid merge into a neighbouring exact polygon rather than
losing their authored solid. The resulting grid union must equal the existing
normalized collision; otherwise the compiler retains its previous partition.
The existing coverage check also verifies the precise partition. A captured
terrace fixture checks matching grid footprints and the clear landing, and a
synthetic holed obstacle checks exact solid/island coverage.

`york-east-riverside-southern-wall-stair-neighbour-placements-vZKHZ7` passes all
32 native directed actor routes and 32 missing/raised terrace rejections.
Full-map `york-riverside-walkway-floor-b3uJu1` retains all 130 passing stair
routes and six control apply/reset checks. These reports finished; no stair
routes were omitted. The wooden stair still uses projected navigation in this
candidate. The separately failed wooden-stair physical correction and broader
floor mesh/placement review remain open; no library asset is published here.

Validation: 188 affected compiler tests, app typecheck, focused lint and the
production build pass. Pipeline typecheck reports the same five pre-existing
test typing errors recorded above, with no new errors in the changed files.

### Southern riverside and wooden stair publication

`stage-york-wooden-stair-floor.mjs` replaces the wooden stair's oversized
navigation envelope with its structural flight footprint. Mesh review improves
from 450/862 supported samples and a 13.658-unit uncovered edge gap to 912/920
samples and a 0.086976-unit gap. Near-floor mesh height residuals range from
about 0.079 to 0.113 units. The first narrower-floor candidate
`york-wooden-stair-floor-JTy8jm` still fails both wooden-stair routes because the
terrace volume overlaps the flight near its upper entrance.

The independently owned terrace and walkway contact corners now meet the stair
edges, with corrections below 0.6 units. A sub-0.001-unit seating correction
makes the flight exactly planar, and its obsolete sloped clearance is replaced
by a clearance derived from that flight. Candidate `york-wooden-stair-floor-kzS32E`
passes all 130 full-York stair routes and six control apply/reset checks.
`york-riverbank-wooden-landing-steps-neighbour-placements-UCiH3n` passes 32 actor
routes at four rotations and two elevations with independently placed walkway
and terrace copies; 64 missing/raised neighbour cases reject.
The same combined descriptors retain all 32 southern-stair routes and 32
missing/raised terrace rejections in `...southern-wall-stair-neighbour-placements-bidpFC`.

`review-staged-receiving-floors.mjs` checks boundaries and interior grid samples
against nearby mesh faces. The terrace has 1,264/1,635 supported samples, a
0.149704-unit maximum edge discrepancy and 0.032518-unit maximum sampled height
error. The walkway has 458/638 supported samples, a 0.092705-unit edge discrepancy
and a 0.050928-unit height error. These are sampled mesh limits, not runtime
connection tolerances or rendered actor certification.

Reviewed stage `york-riverside-reviewed-H72jM4` publishes both stairs, the terrace
and walkway with explicit draft mesh/rendering warnings. Backup:
`york-southern-and-wooden-stairs-publication-20261006`. Fresh published geometry
matches full York and all sixteen native-tested placement exports, excluding
only the updated warnings. All ten saved scenes reopen; only York's scene pins
change. Audit `lift-anchor-support-Ix1scr` finds 13/53 unsupported stairs (York
12/16 and Sherwood 1/1), 1/8 ladders and 9/9 walls. Broader traversal, visual
integration and library coverage remain open.

### Riverbank stone stair publication and physical clipping spikes

Local seam candidate `local-stair-seams-tEKLzq` passes 32 synthetic landing
routes and 64 missing/raised landing rejections (`external-stair-landings-iAEK5t`).
Its flight has 899/919 sampled mesh hits, a maximum uncovered edge distance of
0.576822 units and mesh height residuals between -0.083 and -0.065 units.
Full-map `york-stone-stair-wgUTMw` fails the two stone-stair routes at the upper
terrace contact. `stage-york-stone-stair-contact.mjs` corrects the independently
owned terrace corners by 0.284295 and 0.184350 units. Candidate
`york-stone-stair-contact-j4hxxN` restores all 130 full-York routes and six
controls, but its actual-neighbour fixture `...stone-landing-steps-neighbour-placements-a4tefk`
fails four of 32 routes in the first copy at zero rotation.

Those translated failures expose a zero-width backtracking spike left by
physical collision clipping. Runtime coordinate conversion turns it into an
entrance barrier. Physical obstacle rings now use the existing straight-edge
and spike cleanup, with a captured coordinate regression preserving the actual
solid triangle. No connection tolerance changes. All 153 affected compiler
tests, app typecheck, focused lint and production build pass.

With cleanup, full-map `york-stone-stair-contact-IEVm2R` passes 130 routes and
six controls. Actual-neighbour fixture `...stone-landing-steps-neighbour-placements-hud5S8`
passes 32 routes and 48 missing/raised terrain/terrace rejections. The same
terrace passes 32 wooden-stair routes with 64 rejections (`...-onin3X`) and 32
southern-stair routes with 32 rejections (`...-8xMcwe`). All native audits finish.
Receiving-floor mesh review retains the terrace's maximum edge discrepancy of
0.149704 units and sampled height error below 0.033 units.

Reviewed stage `york-stone-reviewed-nCPbRQ` publishes the stone stair and terrace
with explicit mesh/rendering draft warnings. Backup:
`york-stone-stair-publication-20261006`. Fresh geometry matches full York and
all 24 placement exports, excluding only updated warnings; all ten scenes
reopen and only York's pins change. Audit `lift-anchor-support-zPHA6r` now finds
12/53 unsupported stairs (York 11/16 and Sherwood 1/1), 1/8 ladders and 9/9 walls.
These checks do not establish complete rendered or arbitrary-placement parity.

### Market stair contact and attached landing-wall rounding strips

Unpublished candidate `york-market-stair-contact-P6JmfO` corrects the market
stair's independently owned raised-terrain contact. All 205 sampled contact-strip
points have receiving mesh support. The receiver's collision contours use their
physical height for placement while retaining a separate navigation height.
Full York passes 130 stair routes and six controls. Actual-neighbour fixture
`york-market-southwest-connecting-stairs-neighbour-placements-J54ggQ` compiles
eight translated/rotated/elevated assemblies and rejects 48 missing/raised
landing cases, but initially passes only 24/32 native actor routes.

Native binding exposes a subpixel collision strip along the lower landing seam.
The solid also extends into a large real wall, so comparing its total clipped
area against its overlap with the stair incorrectly prevents seam cleanup.
`attached-landing-seam-strip.json` captures the failing geometry. The regression
first fails on the blocked entrance; local stair-side collision probes now
permit cleanup of the shared rounding strip while preserving the attached wall.
Existing standalone thin-obstacle and landing-side-wall checks still pass.

With the runtime correction, the same actual-neighbour fixture passes all 32
actor routes, and the full candidate retains 130/130 routes. The stair-filtered
native suite passes 67 tests with nine opt-in audits ignored. Full York also
retains all six control apply/reset checks, and the published stone-stair
actual-neighbour fixture `...-hud5S8` retains all 32 actor routes. The game build,
targeted Rust formatting and diff checks pass. The candidate assets are not yet
published; broader placement and visual parity remain open.

The market authoring candidate also reaches floating-point contour-union sweep
limits before its fixed-grid fallback succeeds. Reproduction
`york-market-stair-contact-wJmiUr` retains the 205 supported contact samples.
Using fixed-grid union unconditionally regresses the existing noisy-fragment
outline test by introducing a rounded corner, so that experiment is rejected.
The conditional fallback remains; all 131 contour, gameplay compilation and
surface-placement tests pass. Avoiding the expensive failed sweep without
changing otherwise-valid contours remains open before publication.

### Contour retry review across saved scenes

Unconditional fixed-grid normalization, even after restoring authored vertices,
is rejected. Its ten-scene batch `market-contour-scene-batch-CjhDTa` passes 84
climbing routes and 71 control checks, but changes Nottingham collision
partitioning and makes the stair audit excessively slow. That audit was
intentionally interrupted while testing Nottingham; it is not a passing batch.
Unconditional local-coordinate union also changes successful contours and fails
on a Croisement02 numerical segment. Neither experiment is the final compiler.

The final approach retains successful unions exactly and retries only recognized
clipping numerical failures in a translated coordinate frame. No additional
quantization is applied. The captured regression limits the clipping budget in
an isolated subprocess, proves that direct union fails, and verifies the recovered
covered area without modifying its input. Nottingham's output is exactly equal
to a fresh compilation with the pre-change contour implementation (166 obstacles).

Candidate `contour-retry-scene-batch-sbV9Zi` compiles all ten saved scenes with
the market edits. `...connecting-stairs-neighbour-placements-YkXvwX` passes all
32 actor routes and rejects 48 missing/raised landing cases. All 421 shared tests
pass (one unrelated skip), as do focused lint, app/pipeline typechecks and the
editor production build. Existing test typing was updated for optional diagnostic
messages, physical endpoint unions and assertion-function imports. The retry
still waits for the library's sweep limit on pathological contours; full York
takes about 43 seconds in this run, so export-performance work remains open.

The completed ten-scene native audits pass 288 stair routes (268 seconds), 84
climbing routes and all 71 control apply/reset checks. All report completion;
none claims rendered gameplay parity. The isolated final York check in
`york-market-reviewed-Q6QQEK` also passes all 130 stair routes.

Reviewed publication `york-market-reviewed-Q6QQEK` installs the stair and raised
terrain with explicit draft mesh/ownership/rendering limitations. Backup:
`york-market-stair-publication-20261006`. Only York scene pins change. All eight
fresh placement exports in `...connecting-stairs-neighbour-placements-8xcUcR`
match the native-tested `...-YkXvwX` geometry, excluding only warning text.
Fresh full York in `york-market-published-81ndNG` also matches the tested geometry,
and all ten saved scenes reopen with current pinned descriptors.
Audit `lift-anchor-support-PUizW9` finds 11/53 unsupported stair definitions
(York 10/16, Sherwood 1/1), 1/8 ladders and 9/9 walls.

The next unpublished southeast-wall stair candidate `local-stair-seams-Z3MiAE`
needs a three-unit authoring review bound for its upper landing edge, rather
than the default two. Its flight has 926/927 supported mesh samples, a maximum
uncovered edge distance of 0.027388 units and sampled height residuals between
-0.200 and -0.157 units. Synthetic landing fixture `external-stair-landings-tMnNQm`
passes 32 native actor routes and rejects 32 invalid lower landings. Actual
neighbour/full-map checks and publication remain outstanding.

### Southeast wall stair: physical connectivity and terrain contact

The actual placed-asset fixture initially fails at 90 degrees: grid rounding
pinches the projected flight into two regions touching at one point. Its physical
floor is connected. The compiler now retries disconnected stair assembly using
validated physical navigation, only when subtracting permanent physical collision
still leaves one connected floor. The emitted physical area retains its own
collision identities. Genuine collision-split stairs keep their previous rejection
and best-effort behavior. Captured fixture `near-edge-on-placed-stair.json` fails
with recovery disabled and passes with it enabled.

`...outer-southeast-wall-stair-neighbour-placements-my4Mc1` passes 32 native
routes at four rotations and two elevations, and rejects 16 missing/raised terrain
cases. Full York initially retains two lower-entrance failures in
`york-southeast-stair-5S4CFk`: the terrain-owned boundary stops short of the
corrected stair foot. `stage-york-southeast-ground-contact.mjs` aligns that edge
from `[2466,2284]–[2525,2297]` to approximately
`[2466.109002,2283.416435]–[2525.466293,2294.503614]`, bounded by three units.
The terrain model is a camera-facing backdrop, not a physical terrain mesh.
Its 205 sampled coverage checks only confirm artwork coverage; the publication
warning explicitly retains the unverified rendered/physical contact limitation.
Candidate `york-southeast-ground-contact-WdCTsM` passes 130 full-York stair routes
and all six control apply/reset checks.

All 422 shared tests pass with one unrelated skip, as do focused lint, app and
pipeline typechecks, formatting and the editor production build. Reviewed stage
`york-southeast-reviewed-Ep8hIQ` publishes the stair and terrain definitions with
backup `york-southeast-stair-publication-20261006`; only York scene pins change.
All eight published placement exports in `...-qFbXZU` match the native-tested
`...-my4Mc1` geometry, excluding warning text. Audit `lift-anchor-support-CoaOXb`
finds 10/53 unsupported stair definitions (York 9/16, Sherwood 1/1), 1/8 ladders
and 9/9 walls. This remains short of arbitrary-placement or rendered parity.

Fresh publication batch `york-southeast-published-5c5ZG8` reopens all ten scenes.
York's serialized geometry matches `york-southeast-ground-contact-WdCTsM`, and
the other nine maps match the previously native-tested
`contour-retry-scene-batch-sbV9Zi`, excluding warning text. Comparison uses actual
JSON exports, including their normalization of negative zero.

## York outer east upper stair: independent curtain-wall contacts

The staged `building-242-lift` correction in `local-stair-seams-ZIIUaP`
passes 32 synthetic receiver routes and rejects 64 missing/raised landings.
Its real neighbour is `york-outer-east-upper-curtain-wall`. Initial real-neighbour
fixture `york-outer-east-upper-wall-stair-neighbour-placements-VtHmle` fails all
32 actor routes; full-York candidate `york-east-upper-stair-KEpFDP` fails two of
130. Actors reach the stair sector but stop at collision strips across its ends.

`stage-york-east-upper-stair-contact.mjs` authors the wall's independent opening
and landing contacts from the staged floor and saved editor placements. It reads
only editor/library assets. The wall's two flight clearances match the corrected
floor with a 1/1024-unit local opening margin: exact coincident clipping exposed
a sliver that blocked an endpoint in the second copy. Flight-only candidate
`york-east-upper-stair-contact-S1ziqK` compiles eight placements but still fails
all 32 routes, now at the remaining landing boundary edges. Projecting the wall's
ground clearance and upper floor/clearance contacts onto their respective stair
height seams moves six corners by 0.746–1.053 units, bounded by two units.

Candidate `york-east-upper-stair-contact-X1njp0` produces fixture
`york-outer-east-upper-wall-stair-neighbour-placements-4V0heI`: all 32 native
actor routes pass across two independent copies, four rotations and two heights.
All 48 missing/raised wall or authored-terrain cases reject. The wall opening
remains wall-owned and the test moves each assembly independently.

`review-staged-receiving-floors.mjs --all-surfaces` additionally samples ordinary
floors, not only volume-backed receivers. The candidate flight has 886/970 exact
mesh hits, maximum edge gap 0.326191 and height residual 0.036406 units. The wall
floor has 510/764 hits, maximum gap 0.449199 and residual 0.061213 units. These
are sampled mesh checks, not rendered actor certification. Both definitions
remain unpublished pending full-York and publication checks.

Full-York stage `york-east-upper-stair-contact-vE9N9E` uses identical definitions
and passes all six control apply/reset checks, but retains two lower-entrance
failures among 130 routes. Its upper contact now works. The terrain owner still
has the old lower boundary. `stage-york-southeast-ground-contact.mjs` now also
accepts this reviewed stair assembly, staging terrain points `[2725,1911]` and
`[2761,1918]` at approximately `[2725.141045,1910.268203]` and
`[2761.152464,1917.208955]`. Stage `york-east-upper-ground-contact-uVCfKF` retains
all 205 projected artwork coverage samples. This ground model is a backdrop;
coverage does not certify a physical terrain mesh or rendered contact.

The terrain-corrected candidate passes all 130 full-York stair routes and all
six control apply/reset checks. Reviewed stage `york-east-upper-reviewed-HqMQKz`
publishes the stair, curtain wall and terrain with backup
`york-east-upper-stair-publication-20261006`. Only York scene pins change.
Publication preserves explicit sampled-mesh and rendered-contact limitations.
All eight fresh placement exports in
`york-outer-east-upper-wall-stair-neighbour-placements-3hJRu4` match the tested
`...-4V0heI` serialized geometry, excluding warning text; the 48 negative cases
still reject. All ten saved scenes reopen in the fresh publication check.
Local-anchor audit `lift-anchor-support-oCy317` now finds 9/53 unsupported stair
definitions (York 8/16, Sherwood 1/1), 1/8 ladders and 9/9 walls. Authoring-tool
lint and formatting checks pass; no compiler or runtime code changes are needed
for these asset corrections.

Fresh full-York export `york-east-upper-published-qiTScV` matches the
native-tested terrain-corrected descriptor exactly after excluding warning text.

## York north-garden stair: joined-flight limitation

Candidate `local-stair-seams-OCZPtq` stages both `building-049-lift` and
`building-050-lift` on `york-north-garden-wall-and-stair`. The first has the
external ground entrance; the second has the asset-owned upper landing. Their
shared `joins` anchor connects the flights. The default two-unit upper landing
bound rejects: its two contact corners are 1.535 and 2.727 units from the seam.
An explicit three-unit candidate passes local validation and is mesh-reviewed.

`external-stair-landings-zQzgtL` fails the physical-navigation assertion: after
joining these slightly different floor planes, the compiler reports
`Walkable surface must be planar; split it into planar asset surfaces` and
retains projected navigation. `compilePhysicalStairRegion` currently builds a
single plane for the assembled region. This is a physical traversal limitation,
not evidence that the joined stair works. No definition is published; the next
check must address continuous traversal across the flight planes rather than
counting projected fallback as parity.

The new `robin_level_data::stair_navigation_floor` primitive keeps validated
patch planes and splits route segments at every crossed patch edge. Movement
distance is measured along each 3D flight; unused step distance continues across
the seam. It rejects disconnected patches, unequal shared heights, conflicting
overlaps, unsupported points and concave-floor shortcuts. Five focused tests
cover both travel directions, crossing within one tick, exact seam landings,
collinear seam travel, compatible overlap, four rotations, two translated/raised
copies, zero/invalid budgets and validated JSON round trips. This is height and
distance infrastructure only: live collision authorization remains separate.
The descriptor, compiler, door/landing binding and actor movement loop still
need integration before the north-garden candidate can use it.

Validation: all 91 non-ignored `robin_level_data` library tests pass (eight
data-dependent checks remain ignored), workspace Rust formatting passes, and
the game binary builds successfully. No descriptor schema or runtime actor
behavior changes in this infrastructure step.

Piecewise floor metadata now loads through optional `physical_navigation.floor_patches`.
Admission validates continuous patch heights and coverage against the walking
boundary, allowing missing geometry only behind permanent collision. Rounding
tolerance is applied along real edges, rather than as a total-area allowance
that could hide an unsupported notch. Stair binding caches the validated floor;
actor steps spend their distance across flight planes, refresh their receiving
plane and preserve world-distance animation forecasts. Door handoffs and source
authorization select the local floor. Screen inversion rejects multiple valid
world positions, including a coincident edge-on flight. Replay schema 64 records
the additional raw lift metadata.

The native joined-flight fixture uses slopes four and six across a shared seam.
It passes four complete gate routes across both slopes, with four corresponding
closed-barrier rejections through repeated state toggles. Admission tests reject
height discontinuities, shortened floor coverage and a narrow missing-floor
notch, and retain patch definitions through a raw-lift bitcode round trip.
The physical-stair, stair-navigation and compiled-stair-control suites pass all
48 tests; 29 replay tests and 92 level-data tests pass (eight data-dependent
level-data checks remain ignored). Both editor typechecks and focused shared
schema lint pass. Compiler emission and the garden asset's shared-edge correction
remain pending, so its staged export still falls back to projected navigation.

Workspace Rust formatting, the shared schema's editor formatting check and the
game binary build also pass. Existing planar descriptors retain their current
navigation representation; normal export will opt into patches only after
compiler assembly and its placed-asset regression checks are complete.

### Joined-flight compiler and native integration (2026-10-06)

Editor export now emits piecewise physical stair floors, splits projection edges
at slope changes, and assembles collision independently on each plane. Control
names are remapped across flights after cropping; a crop retaining only one
flight produces the ordinary planar representation. The focused compiler suite
passes 148 tests, including copied/rotated compound lifts, incompatible shared
edges, separate controls and cropping. Both editor typechecks and focused lint
pass.

`shared/test-fixtures/joined-physical-stair.ts` produces the JSON used by the
native joined-stair movement tests. The editor test checks exact serialized
fixture equality. Native admission and complete actor traversal pass: four
open-barrier routes and four closed-barrier rejections across repeated apply/reset
cycles. Rust formatting and the game binary build pass.

The unpublished garden contact candidate is staged by
`refinement/stage-york-garden-joined-seam.mjs` from
`local-stair-seams-OCZPtq` into `york-garden-joined-seam-ONmvxk`. It preserves both
flight slopes and moves their shared edge onto the equal-height line. The
external-landing checker now counts assembled lifts rather than authored flight
segments. `external-stair-landings-HjrSFt` exports sixteen copies at four rotations
and two elevations, rejects 32 missing/raised synthetic landings, and passes all
32 native actor routes. This does not establish full-scene or rendered parity.

Mesh review finds 538/540 sampled hits on the lower flight (maximum edge gap
0.094 units) and 714/811 on the upper flight (maximum gap 2.562 units). The upper
extension needs further mesh-based correction; the candidate remains unpublished.

### Garden stair fitted contacts and draft publication (2026-10-06)

The revised authoring tool fits the upper flight through the lower flight's
shared edge and its own upper landing midpoint, preserving distinct slopes.
The maximum height adjustment is 0.074860 units; upper landing contacts and
receiving-plane metadata move with the floor. `york-garden-joined-seam-okKnUB`
has 528/528 lower-flight mesh hits and 802/803 upper-flight hits, with maximum
edge gap 0.014669 and sampled height residual 0.079796 units. The added authoring
guard's `york-garden-joined-seam-6eKmZO` produces identical edits.

`external-stair-landings-vOq0f4` passes all 32 native routes and rejects 32
missing/raised synthetic receivers. Actual asset copies on editor-authored
terrain in `york-north-garden-wall-and-stair-neighbour-placements-P6vk5D` pass
another 32 routes and reject sixteen missing/raised-terrain cases. The neighbour
checker now counts assembled lifts and centers the garden fixture to avoid
cropping its rotated copies.

The initial full York candidate passed 128/130 routes and six controls. Both
failures were at the lower terrain contact. `york-garden-ground-contact-o2aXay`
aligns the terrain-owned edge from `[3049,292]`–`[3078,278]` to
`[3048.743717,291.454435]`–`[3077.888822,277.763327]`. All 205 backdrop samples
are covered; this is not physical mesh evidence. Full York then passes all
130 routes and all six control apply/reset checks.

Published the stair and terrain with backup
`york-garden-stair-publication-20261006`; only York's scene pins change. All ten
scenes reopen. Eight fresh published placement exports in
`york-north-garden-wall-and-stair-neighbour-placements-DvRcCK` match the tested
geometry. The fresh `lift-anchor-support-sYwn9A` audit leaves 7/53 stair,
1/8 ladder and 9/9 wall definitions with unsupported local anchors. Rendered
contact and broader placement/state coverage remain open.

Fresh full-York export `york-garden-published-ewoJkC` also matches the native-tested
candidate geometry exactly. Authoring scripts pass syntax checks and the owned
changes pass whitespace validation.

### Precinct stair and independent raised receiver (2026-10-06)

`stage-local-stair-seams.mjs` now offers explicit
`--floor-from-receiving-footprint` authoring. It replaces a coarse floor with
that asset's own planar receiving footprint before seating endpoints; it is
not an export fallback. The precinct candidate `local-stair-seams-CbYhGf` uses
this option and a reviewed four-unit midpoint bound. Its synthetic-landings
fixture `external-stair-landings-MNihY4` passes 32 routes and rejects 32 invalid
neighbours. The actual receiver fixture `...neighbour-placements-DeO8xG` fails
all 32 routes, and full York passes only 128/130; all six controls pass.

`stage-precinct-stair-receiver.mjs` authors the matching edge in
`york-cathedral-precinct-raised-terrain`, independently of stair placement.
Precision-only candidate `precinct-stair-receiver-L4Ba0c` still fails moved
traversal. Corrected candidate `precinct-stair-receiver-aJONga` seats receiver
corners by 0.410924 and 2.235306 units. Actual paired copies in
`york-precinct-east-wall-stair-neighbour-placements-BGnR5W` pass all 32 routes,
with 32 missing/raised receiver rejections. Full York passes all 130 routes and
six control apply/reset checks.

Full-surface mesh sampling finds maximum gaps of 0.274033 on the flight,
0.451621 on its upper landing and 0.281792 on the receiver; corresponding height
errors reach 0.114944, 0.055993 and 0.061466 units. These remain draft limits;
rendered verification is open. The two assets are published with backup
`york-precinct-stair-publication-20261006`. All ten scenes reopen. Eight fresh
published exports in `york-precinct-east-wall-stair-neighbour-placements-vGatk4`
match native-tested geometry. Audit `lift-anchor-support-B37lQ0` leaves 6/53
stair definitions with unsupported local anchors (York 5/16, Sherwood 1/1),
plus the unchanged 1/8 ladder and 9/9 wall definitions.

Fresh full export `york-precinct-published-S7G7nd` matches the native-tested York
candidate geometry exactly. Authoring scripts pass syntax and whitespace checks.

### South-city-gate east stair publication (2026-10-06)

`local-stair-seams-tJzoY5` corrects both endpoints and the asset-owned upper
landing. `external-stair-landings-5OjiXC` passes all 32 moved native routes and
rejects 32 missing/raised receivers. Full York passes all 130 stair routes and
six controls, including the lower physical receiver bound above ordinary ground
navigation. No neighbouring geometry change is required.

`local-stair-seams-FX2PTQ` adds placement ground height `90.00057027945812` and
an explicit mesh-review warning. Its geometry equals the tested candidate.
New editor-terrain drops in `local-stair-placements-1lJlRr` pass sixteen routes
at four rotations and two elevations, and reject eight raised placements.
Flight mesh sampling finds 731/733 hits, maximum gap 0.239863 units and maximum
absolute height residual 0.076586 units. Rendered traversal remains open.

Published the stair with backup `york-southgate-stair-publication-20261006` and
refreshed York's pins. All ten scenes reopen. Eight fresh terrain-drop exports in
`local-stair-placements-03EuDK` match tested geometry. Copied fixture
`external-stair-landings-hdXOJC` shifts its placement after adopting the authored
ground height; its fresh native check passes all 32 routes. The fresh
`lift-anchor-support-SNag0C` audit leaves 5/53 stair definitions with unsupported
local anchors (York 4/16 and Sherwood 1/1); ladder/wall counts remain 1/8 and 9/9.

Fresh full-York export `york-southgate-published-dtaqwU` matches the native-tested
candidate geometry exactly; the added placement height affects new drops only.

### West-city-wall stair and elevated receiver contact (2026-10-06)

`local-stair-seams-GyRGQN` corrects both local contacts. Mesh review finds
839/839 flight hits with maximum absolute height residual 0.105429 units.
`local-stair-seams-uYbT9q` adds placement ground height `90.00101` and a draft
mesh-review warning; its placed geometry is unchanged. Copied fixtures
`external-stair-landings-RHJr8H` pass all 32 routes and reject 32 missing/raised
receivers. New terrain drops `local-stair-placements-r8YNRE` pass sixteen routes
and reject eight raised placements.

Full York initially passes 128/130 routes and all six controls. Both failures
occur at the lower handoff, where ordinary ground navigation is projected under
the elevated physical receiver. The ground-contact authoring tool now supports
an explicit receiver altitude; its default zero-altitude behavior is retained.
`york-westwall-ground-contact-d4jF0z` moves the terrain-owned edge from
`[300,532]`–`[317,551]` to `[301.611948,530.501163]`–`[318.969545,549.168658]`.
All 205 sampled backdrop points are covered. Full York then passes all 130 routes
and six control apply/reset checks.

Published the stair and terrain with backup
`york-westwall-stair-publication-20261006`. All ten scenes reopen. Eight fresh
terrain-drop exports in `local-stair-placements-cvjKtI` and eight copied exports
in `external-stair-landings-ltoKyT` exactly match native-tested geometry. Audit
`lift-anchor-support-iAh7Gd` leaves 4/53 stair definitions with unsupported local
anchors: three in York and one in Sherwood. Ladder/wall counts remain 1/8 and 9/9.

Separate Sherwood investigation stages `local-stair-seams-b0YsHF`; it is not
published. Its shared-node mesh review shows large discrepancies, including
zero stair-flight sample hits. Correct surface ownership/mesh evidence needs
review before treating its successful descriptor construction as traversal parity.

Fresh full export `york-westwall-published-7E1mLP` matches the native-tested
candidate exactly. The contact-authoring tool passes syntax and whitespace checks.

### Multi-entrance stair candidates (2026-10-06, unpublished)

West-lane candidate `local-stair-seams-SmR0l9` retains eight entrances.
`external-stair-landings-9esUa3` passes all 896 directed routes across sixteen
copies and rejects 64 missing/raised synthetic receivers. Flight sampling finds
1,294/1,295 mesh hits, maximum gap 0.062427 and height residual 0.094967 units.
The broader contour review samples a maximum edge gap of 0.251467 units.
Full York exposes 54 failing handoffs out of 130 routes; only two of the
west-lane lift's 56 directed routes pass in that initial candidate.

`stage-west-lane-stair-contacts.mjs` authors receiving corrections independently
in `york-terrain` and `york-east-bridge-raised-terrace`. Candidate
`west-lane-stair-contacts-98Papl` moves the lower ground edge by 0.309303 units
and the terrace edge by 1.039790 units. The physical-receiver mesh review in
`west-lane-physical-receivers-VUrFKg` finds terrace maximum gap 0.539013 and
height error 0.098886 units. A separate backdrop-coverage review confirms all
205 ground-strip samples; this is not physical support evidence. This candidate
passes all six controls but still fails 36 of 130 full-York routes. Three lower
entrances miss the slightly angled physical terrace boundary.
Candidate `west-lane-stair-contacts-9A7SvO` additionally aligns both the floor and
explicit receiving volume on `york-southeast-riverside-raised-terrace`, moving
two corners by 0.107700 and 0.085600 units. All 130 full-York native stair routes
and six control apply/reset checks pass.
The new `exported_stairs_support_complete_sprite_actor_routes` audit also passes
all 130 routes using RobinTown's complete animation profile (73 seconds), with
`complete_sprite: true` in `actor-stair-sprite-route-report.json`. It uses the
same arrival, sector/layer and receiver checks and remains a simulation test,
not rendered compositing verification. Engine test compilation, game build and
Rust formatting checks pass.
Mesh review `west-lane-physical-receivers-99PUdW` finds 1,327/1,635 lower-terrace
hits, maximum edge gap 0.149704 and height residual 0.032518 units. The stair and
upper terrace retain their earlier sampled gaps. All 205 ground backdrop samples
are covered. Real paired-terrace fixture
`york-west-lane-access-steps-neighbour-placements-BeC1JM` compiles sixteen copies
at four rotations and two elevations and rejects 64 missing/raised neighbours;
all 896 complete-profile native routes pass.
Actual-neighbour fixture
`york-west-lane-access-steps-neighbour-placements-NVq5B7` exports sixteen copies
and rejects 48 missing/raised receivers or terrain; its first 448 native routes
pass. That synthetic-frame process exits with status 143 before the elevated
cases complete; its report remains unfinished. A separate complete-profile
audit now passes all 896 routes across the eight exports.

Bridge candidate `local-stair-seams-4oe3PY` retains both flights and all eleven
entrances. The external-landing fixture now gives disconnected coplanar receiving
islands separate navigation regions while retaining shared asset ownership.
`external-stair-landings-sroZbJ` passes all 800 directed routes and rejects 64
missing/raised receiving cases. Flight mesh hits are 883/885 and 836/846;
maximum gaps are 0.068722 and 0.282337 units. Combined full-York candidate
`york-multi-entrance-contacts-O4gMqo` contains both stair candidates and the two
receiving corrections; it predates the lower physical-terrace correction.
Replacement `york-multi-entrance-contacts-dvaYCq` combines all five edited assets;
compilation and six control checks pass, but 42/130 native stair routes fail.
Twenty-four failures involve the first flight's three upper entrances; eighteen
involve the second flight's three lower entrances. The west-lane routes pass.
The bridge's actual-neighbour fixture
`york-stone-river-bridge-and-approach-stairs-neighbour-placements-k2Zlar` compiles
sixteen copies with both terraces and authored terrain and rejects 80 invalid
neighbour/terrain cases. Removing terrain preserves the other flight in each
copy, whose approaches belong to the physical terraces. The first placement
fails 84/100 native routes, so this candidate cannot be published.

`stage-york-bridge-stair-contacts.mjs` produces `york-bridge-stair-contacts-P0lynX`:
the separate upper terrace owns contact corrections of 1.106673 and 0.388693
units. The bridge owns a new explicit lower-deck floor and matching receiving
volume, with corners adjusted by 0.216471 and 0.392052 units. Other part solids
are copied unchanged into explicit asset volumes, preserving sight order; no
level data is used. The lower deck no longer needs terrain beneath it to supply
walking support. Mesh review `york-bridge-receiver-mesh-6rurWV` finds 1,103/1,127
lower-deck hits, maximum gap 0.216415 and height residual 0.032243 units; the
upper terrace retains a maximum gap of 0.539013 units.
Actual-neighbour fixture
`york-stone-river-bridge-and-approach-stairs-neighbour-placements-uUptc0` compiles
sixteen copies without underlying terrain and rejects 64 missing/raised
terraces. All copies retain both flights and all eleven entrances. All 800
complete-character-profile routes pass. Full York passes all 130 complete-profile
stair routes, 40 climbing routes and six control checks.
Other bridge obstacle shapes/flags and existing movement contours, clearances,
doors, lifts, jumps and interiors are verified unchanged by the deck migration.
Combined moved fixture
`york-stone-river-bridge-and-approach-stairs-neighbour-placements-lND6cV` includes
the west-lane asset alongside both bridge flights and their shared terraces.
Eight exports retain all entrances and reject 64 missing/raised terrace cases.
The initial run passed all 848 ground-height complete-profile routes before
interruption. The four remaining elevated exports were copied unchanged into
`york-combined-elevated-pa7oczf4`; all 848 elevated routes pass (359.14 seconds).
Together this verifies all 1,696 routes. These descriptors predate physical-wall
compilation; fresh full-scene regression remains required before publication.
Nothing in this
section is published or claimed as full-scene parity.

### Sherwood shared-node flight review and draft publication (2026-10-06)

The mesh reviewer now gives flights sharing a node separate sample/image names,
so one flight cannot overwrite another's evidence. It additionally records the
nearest initially visible mesh in the assembled asset, without treating an
overhead surface as proof of walking support. On `local-stair-seams-b0YsHF`, only
194/802 stair samples and 154/839 ladder samples have assembled mesh within two
height units; some residuals exceed 45 and 35 units respectively. The stair is
therefore not visually verified simply by including neighbouring component meshes.
The staged stair does pass all sixteen complete-profile actor routes in
`local-stair-placements-zCmumg`, covering four rotations and two elevations.
The neighbouring ladder also passes all sixteen complete-profile routes.
Full Sherwood in `sherwood-oak-stair-full-9faYPC` passes two stair routes and ten
climbing routes. Its control audit reports zero transitions and fails its
nonempty-coverage assertion; Sherwood has no controls, so this is not recorded
as a passing control check.

`local-stair-seams-ErXHU8` reproduces the tested geometry exactly and adds an
explicit draft warning about the substantial visual gaps. Published this stair
correction with backup `sherwood-oak-stair-publication-20261006`; the existing
ladder definitions remain unchanged. All ten scenes reopen. Eight fresh
published placements in `local-stair-placements-sZTxB8` and full Sherwood in
`sherwood-oak-published-WphOAd` exactly match the native-tested geometry.
Anchor audit `lift-anchor-support-hSBiol` now finds 3/53 unsupported stair
definitions, all in York; ladder/wall counts remain 1/8 and 9/9. None of these
navigation results certifies the central oak's visible geometry.

### Physical wall navigation and triangulated approaches (2026-10-06)

The compiler previously attempted physical navigation only for stairs and ladders.
Walls now use the same planar-flight, exact-contour and supported-landing checks.
The native descriptor validator accepts wall-owned physical navigation; wall
climbing orders retain their world destinations. Explicit physical approaches
remain authoritative during loading: applying a separate screen-space wall
offset made the gate assertion disagree with the actual movement endpoint.
Descriptors without physical navigation retain the existing approach adjustment.

The staged Lincoln inner gatehouse in `local-stair-seams-kEIDBM` initially gained
physical navigation in six of eight placements. The two unrotated failures had
their outside point and seam on different triangles of the same flat terrain.
Checking support for each anchor fixes this without changing height tolerances.
Tests cover split terrain support, removal of outside support, raised landings,
fractional rotated contours, and hole edges versus hole interiors for both
ladders and walls. `local-stair-placements-kJTt0J` now retains physical navigation
in all eight placements and passes all sixteen complete-profile climb routes.
The gatehouse asset remains staged; no publication is implied by these results.

The updated changing-climb fixtures cover both wall-top door types at four
rotations and copied independent controls. Native collision/pathfinder, actor
traversal, mid-climb closure and reopening checks pass. Late reopening waits for
actual route failure within the bounded simulation, because wall animation
profiles have different numbers of stationary frames. The copied-control audit
and fractional ladder exit also pass. The regular compiled-navigation suite
passes 47 tests (14 opt-in audits skipped).

Editor validation passes 874 tests with two skips; all 195 focused compiler/export
tests pass, as do editor typechecking and focused lint. Three stale native fixture
snapshots were refreshed alongside the wall fixtures: the piecewise stair floor,
a redundant ladder obstacle closing vertex, and the spline contour-repair warning.
All ten maps recompile in `saved-map-exports-iEv81a`; all 84 climbing routes and
71 apply/reset controls pass. All 288 full-profile stair routes pass (305.49 seconds).
The game build and Rust formatting pass.

Full Lincoln candidate `lincoln-physical-wall-full-0oj2Up` retains all eight
passing climb routes, but the gatehouse still falls back to projected navigation.
Its lower seam lies 2.066150 units outside the separate inner-bailey plateau's
edge from world `[2256.0005,1499]` to `[2306.9995,1506]`. The outside anchor is
supported. Correcting that receiving asset, with mesh and moved-neighbour checks,
is required before publishing the gatehouse; increasing compiler tolerance would
conceal the missing contact.

### York multi-entrance publication (2026-10-06)

Fresh full York in `york-physical-wall-regression-USE3kB` exactly matches the
tested `york-bridge-stair-contacts-P0lynX` geometry apart from warnings. With the
updated runtime it passes all 130 full-profile stair routes (158.52 seconds),
40 climbing routes and six controls. The combined bridge/west-lane placement
matrix completes all 1,696 routes, with both separate west-lane arrangements
also passing 896 routes each and the bridge arrangement passing 800.

`york-multientrance-reviewed-d2qRoP` adds quantitative mesh-review warnings
without changing gameplay geometry. Published all five reviewed definitions
with backup `york-multientrance-publication-20261006`, updating York's saved pins.
Fresh combined placements in
`york-stone-river-bridge-and-approach-stairs-neighbour-placements-hjZB9y` exactly
match all eight tested exports, and reject all 64 missing/raised terrace cases.
All ten saved scenes reopen. Full published York in
`york-multientrance-published-W7LKhw` exactly matches the native-tested geometry.
Audit `lift-anchor-support-pdxfth` reports 0/53 unsupported stair definitions,
1/8 ladders and 9/9 walls. Local anchor support is not full placement, animation,
rendering or gameplay certification.

### Lincoln gatehouse and independent plateau contact (2026-10-06)

`stage-lincoln-gatehouse-contact.mjs` authors a candidate from the staged
gatehouse and current asset definitions. It gives the separate inner-bailey
plateau its own walkable top, preserves its collision flags, light order and
material ownership, and retains its movement exclusions both below and on top.
Two plateau edge vertices move 1.725819 and 2.501154 units to the climbing seam.
The compiler receives only these asset definitions and current placements.

Candidate `lincoln-gatehouse-contact-qMf4we` passes all sixteen full-Lincoln
stair routes, eight climbs and eleven apply/reset controls. The isolated actual
asset pair in `lincoln-inner-gatehouse-neighbour-placements-n2ALGE` contains two
copies at four rotations and two elevations, without terrain borrowed from the
source scene. All 32 missing/raised neighbour cases reject. Initially four of
32 climbing routes failed on the second copy at 37 degrees: its upper landing
was rejected for a 0.001143-unit height difference caused by f32 edge rounding.

Landing binding now propagates coordinate rounding through the floor gradients
when checking a shared edge. The new small native fixture reproduces that case
and still rejects a 0.02-unit height gap. All 32 actor routes now pass. Eleven
landing-binding tests, all 22 stair-navigation tests, and 47 regular compiled
navigation tests pass (14 opt-in audits skipped). Full Lincoln is rechecked with
the corrected runtime. An overly broad `exported_` test invocation also selected
six unrelated scenery/probe audits without their required fixtures; that combined
invocation is not recorded as a passing suite. Its relevant stair, climbing,
copied-control and reopening checks pass.

Mesh review finds 1,617/2,412 supported samples over the plateau's coarse floor,
maximum uncovered distance 118.198667 and height residual 0.145244 units. Its
edited gatehouse seam has 11/41 exact hits and maximum gap 2.968993 units. These
limitations are explicit draft warnings; headless navigation is not a visual
certification. The gatehouse flight retains its earlier 0.133-unit sampled
edge-gap warning and unverified rendered integration.

`lincoln-gatehouse-reviewed-Kp16J1` adds those warnings without changing geometry.
Published both definitions with backup `lincoln-gatehouse-publication-20261006`.
Audit `lift-anchor-support-ugbdci` reports 0/53 unsupported stair definitions,
1/8 ladders and 8/9 walls. All eight fresh published placements in
`lincoln-inner-gatehouse-neighbour-placements-GBLRT1` and full Lincoln in
`lincoln-gatehouse-published-9tYt5l` exactly match the native-tested geometry.
All ten scenes reopen. The game build, Rust formatting and patch whitespace
checks pass.

### Lincoln east curtain wall climb publication

Candidate `local-stair-seams-XCE5iB` corrects `building-157-lift` seams in
`lincoln-east-curtain-wall-middle`. Maximum flight boundary displacement is
0.443668 units. The lower landing remains an external placement receiver; the
upper landing belongs to this asset. No compiler tolerance was changed.

`local-stair-placements-asEKSB` passes 16 complete-profile stair routes and
16 climb routes across four rotations and two elevations. All eight raised
entrance checks reject disconnected geometry. Full Lincoln candidate
`saved-map-exports-lCuUas` passes 16 stair routes, eight climbs and eleven
control apply/reset checks. These are independent placements, not simultaneous
copy-isolation or rendered animation tests.

Mesh review finds only 141/542 wall flight samples hit visible geometry, with
maximum uncovered distance 9.340554 units and sampled mesh-height residuals
from -22.443323 to 8.678490 units. The existing stair mesh warning remains.
`local-stair-seams-GpARjD` adds the wall warning with identical geometry.
Published with backup `lincoln-curtain-wall-publication-20261006`.

Fresh published full Lincoln `saved-map-exports-MpYOO0` and eight placements
`local-stair-placements-qav9un` exactly match the native-tested descriptors
apart from warnings. Anchor audit `lift-anchor-support-if3ARh` now reports
0/53 unsupported stairs, 1/8 ladders and 7/9 walls. Those counts do not certify
full map parity. The saved-map checker accepts repeatable `--map=NAME` options
for focused scene regressions, rejects unknown names, and records the selection
in its diagnostics; omitting them continues to check every saved scene.

### Unpublished keep-annex climb clearance investigation

New-map placement remains the acceptance criterion. The saved Lincoln scene is
only a regression example, and a passing round trip cannot override failed
rotated placements.

The neighbour checker now supports `lincoln-keep-annex` with the independent
`lincoln-courtyard-shed` sloped receiving roof. Candidate
`local-stair-seams-92Ritd` seats the lower seam against that asset-local roof
plane. Its two-copy test `lincoln-keep-annex-neighbour-placements-113Sla`
rejects all 32 missing/raised receiver cases, but all 32 actor routes stall:
the recovered flight contour has insufficient internal clearance. A later
single-map trace overwrote that directory's actor report; the complete initial
failure log is `/tmp/lincoln-annex-113Sla-climbs.txt`.

Candidate `local-stair-seams-AKCtuG` reconstructs the floor using the asset's
existing receiving footprint and declared plane. The authoring command now
allows at most 0.001 unit of footprint-to-plane correction and records the
maximum; this candidate needs 0.000924. This does not change runtime or compiler
tolerances. Full Lincoln `saved-map-exports-DZ3UyU` passes 16 stair routes,
eight climbs and eleven control checks. However, the two-copy new-map test
`lincoln-keep-annex-neighbour-placements-jlgO6X` passes only 24/32 actor routes:
all eight routes at 90 degrees fail across both elevations and both copies.
All 32 missing/raised receiver checks reject. This candidate is not published.

Visible coverage is also unresolved: 513/776 flight samples hit the mesh,
maximum uncovered distance is 11.387280 units, and sampled height residuals
range from -71.778567 to 87.749589 units. Next work must resolve the rotated
actor clearance and review the visible climbing surface, rather than accept
the unchanged-layout success or widen the contour merely to pass the test.

Follow-up measurements on the 90-degree export identify a genuine clearance
deficit: the opposed upper corridor edges are 10.435697–10.500689 units apart,
while the actor's effective rectangular footprint spans 10.582686 units along
their normal. The narrow section therefore cannot accommodate that actor at
this placement. Collision rejection is retained. A focused 90-degree native
trace is in `/tmp/lincoln-annex-rotated-trace.txt`; it overwrote the candidate's
actor report with the selected map, so use `/tmp/lincoln-annex-jlgO6X-climbs.txt`
for the complete eight-placement run.

The independent-neighbour checker also covers
`york-precinct-southwest-wall-ramp` with
`york-cathedral-precinct-raised-terrain` and newly authored ground. Candidate
`local-stair-seams-TsXSTb` corrects the climb floor seams, but the upper receiving
surface does not reach the corrected seam. The test rejects projected fallback
and now includes the compiler warnings in that assertion. No publication or
actor success is claimed for this candidate. The receiving asset's contact
needs explicit authoring before the moved/copied pair can be accepted.

### Steep receiving-plane validation and precinct contact candidates

The copied precinct pair exposed a general loader error: receiving-plane
anchors and polygon vertices round independently to f32. One exported vertex
had 0.002659 height residual against a 0.002385 fixed coordinate tolerance;
another translated copy had 0.005638 residual against 0.004310. These are
valid authored planes. The loader now bounds the scalar triple-product error
using half-ULP uncertainty in all anchor and vertex coordinates, including the
uncertainty in the plane normal. The existing coordinate bound remains as a
minimum. No movement query, clearance, or door matching tolerance changes.
The new loader regression accepts the exported steep plane and rejects
deliberate positive and negative 0.1-unit height mismatches. All 93 regular
level-data tests pass (eight opt-in checks skipped), as do 47 regular compiled
navigation tests (14 opt-in checks skipped), game build and Rust formatting.

`stage-york-precinct-wall-contact.mjs` authors an unpublished paired candidate.
It moves two walkable boundary vertices by 0.991859 and 2.634988 units and two
receiving-footprint vertices by 0.075348 and 0.315463 units. The candidate also
authors a climb-owned upper-height clearance from its existing solid footprint;
it does not clear solids owned by other placed assets.

The earlier navigation-only contact `york-precinct-wall-contact-o4sn0i` compiles
eight two-copy placements with 48 negative disconnect checks, but its upper
receiver does not bind in the runtime. Full York `saved-map-exports-No971Z`
passes 130 stair routes, 40 climbs and six control checks with the loader fix;
that does not establish independent asset usability. The precise receiver
candidate `york-precinct-wall-contact-qOsMIo` resolves binding but still fails
actor routes; its copied test is `...neighbour-placements-rDqzbA`. A clearance
restricted to the flight (`york-precinct-wall-contact-stsFP0`) also fails all
32 routes in `...neighbour-placements-1LeDqc`. These definitions are not published.

The climb mesh review has 808/811 hits, maximum uncovered margin 0.011341 units
and height residuals from -0.267647 to 0.161528. Receiving-floor review has
4732/4944 supported samples, maximum gap 0.281792 and height residual 0.061466.
Draft warnings retain these limitations; neither headless routes nor sampled
mesh coverage substitute for rendered gameplay verification.

The remaining obstruction was the climb asset's separate crest `building-877`,
not the flight solid `building-109`. Final authoring stage
`york-precinct-wall-contact-Sh3UW5` retains only the crest's upper-height opening.
It passes all 48 missing/raised ground and platform checks. Its eight two-copy
exports `york-precinct-southwest-wall-ramp-neighbour-placements-AKReGD` exactly
match native-tested `...neighbour-placements-tACaik`, which passes all 32 routes.
Full final York `saved-map-exports-VQbzev` exactly matches tested
`saved-map-exports-cWDaHS`: 130 complete-profile stair routes, 40 climbs and six
control apply/reset checks pass. No source-scene neighbour is needed beyond the
explicitly placed receiving platform and authored ground.

Published both definitions with backup `york-precinct-wall-publication-20261006`.
Post-publication anchor audit `lift-anchor-support-PpP9Ao` reports 0/53 stairs,
1/8 ladders and 6/9 walls with unsupported anchors. The loader fix is committed
as `a53ea3ceb`. The unresolved annex clearance and other remaining parity work
are not certified by these results.
Fresh published full York `saved-map-exports-YAAF3w` and eight copied placements
`...neighbour-placements-hO7bfM` exactly match their tested descriptors. All ten
saved scenes reopen with valid descriptor pins. Authoring-script formatting and
patch whitespace checks pass.

### Northern York climb and independent boundary-wall opening

Candidate `york-northern-climb-contact-9u7SLw` corrects the northern climb,
its independently owned curtain-wall receiving contact, and the neighbouring
boundary wall's existing climb clearance. The authoring helper records all
changes in asset-local coordinates; compilation does not read the saved York
scene to repair connections.

The earlier two-asset candidate `york-northern-climb-contact-6RJXyE` passed
32 copied actor routes but lost its climb in full York. Collision subsets
`lift-neighbour-collision-kpdm9v` isolate the boundary wall as the cause: adding
that asset alone changes one lift to zero. Its clearance still followed the
previous sloped flight. Updating the boundary-owned clearance to the corrected
flight polygon and plane preserves collision outside that opening and does not
change compiler plane-matching tolerances.

The three-asset placements `...northern-wall-stair-neighbour-placements-3NTNAb`
pass all 32 complete-profile actor routes: two copies, four rotations
(0/37/90/180 degrees), and two elevations (0/40). All 32 missing/raised
receiving-wall controls reject unsupported connections. Removing either
optional boundary-wall copy preserves both climbs at all eight placements.
Full York `saved-map-exports-3IZwmY` passes 130 stair routes and six controls,
but fails both northern climb directions (38/40 climbs pass). This candidate
is not published. The runtime trace reports a 0.047529-unit upper support gap.
Both the boundary wall's flat upper clearance and the climb solid's own crest
clearance still describe the previous receiving contour. Their collision can
survive rounding at the saved placement even though the copied placements pass.

Final candidate `york-northern-climb-contact-yKyoBU` updates these existing
upper openings in each asset's own frame. No runtime tolerance changes. The
placement check now also tests the saved origin's fractional Y offset at all
four rotations and both elevations. `...neighbour-placements-750CQz` passes
64 complete-profile actor routes and 64 missing/raised receiver controls;
removing either optional boundary-wall copy preserves support. Diagnostic
subsets `lift-neighbour-collision-XM39Za` pass fourteen actor routes. Two cropped
terrace subsets cannot compile and remain explicitly excluded from this actor
manifest; they do not count as passing full-map coverage.

Final full York `saved-map-exports-G9UysD` passes all 130 stair routes,
40 climb routes and six control apply/reset checks. All three definitions are
published with backup `york-northern-climb-publication-20261006`. All ten scenes
reopen with valid descriptor pins. Sixteen fresh published placements
`...neighbour-placements-t9S2Ud` exactly match the native-tested candidate.
Post-publication anchor audit `lift-anchor-support-EbcKcW` reports unsupported
anchors in 0/53 stairs, 1/8 ladders and 5/9 walls. Fresh published full York
`saved-map-exports-L4Ocfa` exactly matches the native-tested candidate.
Fresh mesh sampling retains lower-landing gaps up to 0.241 units, flight gaps
up to 0.163 units and receiving-wall gaps up to 0.413 units; height residuals
are at most 0.029, 0.023 and 0.062 respectively. Rendered integration remains
unverified and the asset definitions retain draft warnings.

A fresh index and saved-scene audit finds 38 missing gameplay definitions among
1,297 assets. Four are placed in current Croisement03: ferns 35/76, the stream
fallen log and timber bridge. Placement auditing must use the stored document's
`placements[].assets`; reconstructed editor groups do not have an `assets`
field. Of the 38 missing definitions, 36 have all
component IDs represented in gameplay-bearing assets; the two unmatched assets
are the Leicester background and Nottingham terrain ground. Component coverage
alone does not establish transferable local gameplay ownership.

### Central York gable-house climb candidate

The next independent assembly is `york-central-lane-stone-gable-house` and
`york-central-lane-timber-lean-to`. The lower entrance belongs to authored ground
at world height 90.00101 in the saved placement. The upper entrance lies on the
lean-to's sloped roof, whose plane in the climb frame is
`[0.24438778094190522, -0.7187429957556019, 111.30475209233093]`.

Unpublished `local-stair-seams-1qvQx4` seats the flight against those two planes.
The upper midpoint moves 4.616657 units, recorded under an explicit authoring
bound of 4.7; default limits and runtime matching tolerances remain unchanged.
The paired placement check rejects projected fallback because the roof contour
does not reach that physical seam. This requires receiving-asset geometry review
before native actor checks or publication. Mesh review supports only 205/821
flight samples, with maximum uncovered distance 7.303562 and height residual
0.388868 units. This candidate is not certified for new-map use.

The paired authoring helper `stage-york-gable-roof-contact.mjs` reconstructs the
roof contour from its receiving footprint and seats the contacting edge against
the climb plane (0.064142 and 2.110133-unit vertex changes). Initial paired
candidate `york-gable-roof-contact-S1toGJ` compiles eight copied placements and
passes 48 missing/raised ground and roof checks, but fails all 32 actor routes.
The runtime rejects an additional shared boundary where the roof and lower
flight have different heights. This candidate remains unpublished; full York
`saved-map-exports-U6Ab0E` only establishes successful compilation.

Reconstructing the flight from its own receiving footprint requires a measured
0.001142412-unit seating correction to its declared plane. The authoring helper
now accepts an explicitly reviewed `--footprint-height-limit` only with footprint
reconstruction; its default remains 0.001. Stage `local-stair-seams-uyZmNW` uses
0.0012. Compiler and runtime height tolerances are unchanged. The reconstructed
flight has 252/848 supported mesh samples, maximum gap 7.261157 and height
residual 0.391348. The paired roof has 775/968 supported samples, maximum gap
2.107148 and height residual 0.057636.

Candidate `york-gable-roof-contact-HTLHQ9` also retains a roof-owned movement slab
at the corrected receiving footprint. One copied placement exposed a floating
polygon-intersection failure between this slab and its height slice. The
compiler now retries that specific ring-stitching error with fixed-point
intersection at scale 2^32. Coarser scale 2^20 created a spurious collision strip
at the tangent edge and is deliberately not used for this physical slice.
The captured `coincident-roof-slab-slice.json` regression fails before this fix
and passes afterward, checking blocked slab contact and clear positions above
and below it. All 162 affected compiler/collision tests, both typechecks and
focused lint pass. A pre-existing optional assertion-message type error in the
asset compiler test was corrected without changing its assertion.

With this compiler fix, `...gable-house-neighbour-placements-8Mxjjt` compiles all
eight placements and passes 48 disconnect controls, but still fails all 32
actor routes. No house/roof asset definition has been published from this work.

Landing validation also missed permanently blocked contacts on the flight side:
it previously excluded only the landing's own permanent collision. A regression
with an L-shaped flight fails before the runtime fix and passes afterward.
Validation now excludes inaccessible shared-edge portions blocked on either
side while retaining the collision itself. Removable barriers, partial coverage
and a real 0.02-unit uncovered gap still reject incompatible heights.

The initial runtime change passes all 22 stair-navigation tests, 47 regular
compiled-navigation tests, and published York's 130 stair routes and 40 climbs.
The house candidate remains rejected at an exposed shared-edge point with a
1.595225-unit height mismatch. A subsequent attempt to exclude coordinate-
rounding fragments did not fix that candidate and regressed two published York
stair routes. That extension was removed; no geometric or height tolerance is
loosened. Final rebuild, formatting, all 22 stair-navigation tests, 47 regular
compiled-navigation tests and published York's 130 stair routes and 40 climbs
pass after removing that extension. The final native logs use the
`york-L4Ocfa-final-flight` prefix.

The next unpublished asset candidate `york-gable-roof-contact-EyKTvB` explicitly
excludes the separate climb footprint from ordinary roof walking. This keeps
the explicit entrance as the connection between the two floors. Its paired
exports `...gable-house-neighbour-placements-ROF3Uy` pass all eight compilation
placements and 48 disconnect controls, but all 32 actor routes still fail.
The candidate remains unpublished pending diagnosis of the blocked connection.

Shared-edge collision exclusion now uses the existing coordinate precision for
independently rounded collision and flight contours. Only mismatching contacts
are clipped this way; already matching seams and actual movement collision stay
unchanged. The regression includes a one-ULP collision edge offset, removable
barriers and a real 0.02-unit uncovered gap. All 22 stair-navigation tests, 47
compiled-navigation tests, and published York's 130 stair routes and 40 climbs
pass (`york-L4Ocfa-buffer` native logs).

The zero-degree `ROF3Uy` trace now binds both receiving floors for both copies,
resolving the previous false exposed contact. All 32 candidate actor routes
still fail, both with the roof cutout and without it (`8Mxjjt`). An independent
polygon check of the traced support and the actor's effective 10-by-4 footprint
finds about 8.512 square units unsupported at the upper inside anchor, plus
1.470 square units overlapping the slab collision. Its middle anchor also has
about 2.599 unsupported and 1.571 blocked square units. The lower anchors have
only subpixel representation strips and no slab overlap. These measurements
identify an asset support/collision problem beyond landing binding; they do
not authorize padding the floor or removing the roof's solid volume.

The movement-box review confirms that climbing posture does not select a smaller
box: the posture-taking accessor returns the same box, as does the runtime.
No footprint reduction or entrance expansion was applied to the house candidate.

The current authoring recipe was replayed against the footprint-reconstructed
flight (`local-stair-seams-uyZmNW`), producing `york-gable-roof-contact-dEjYkG`.
Fresh `york-central-lane-stone-gable-house-neighbour-placements-5s3m76` compiles
eight placements, rejects 48 disconnected neighbours and still fails all 32
complete-profile actor routes. Failed native audit results now retain their
bound physical navigation and initial motion states, once per failing sector.
`uv run --with shapely python refinement/audit-lift-support.py REPORT --file FILE`
measures inside/middle footprint support independently with polygon operations.
It subtracts each landing's own holes and active collision before combining
support, and measures active flight collision separately. It does not repair
rounding cracks, validate mesh contact or certify a complete route.

The zero-degree contact report reproduces 8.511748 unsupported and 1.469696
blocked square units at the first copy's upper inside point; the midpoint has
2.598869 unsupported and 1.570729 blocked square units. Its flight spans heights
90.000–225.005. Extrapolating the flight plane over the inside point's full
effective footprint reaches 263.586; the midpoint's reaches 246.304. This
identifies support extending beyond the finite climbing surface, not permission
to extend that surface or shrink the actor. The earlier non-reconstructed flight
candidate (`...neighbour-placements-1DspZ5`) has no slab overlap at the upper
inside point, but still lacks about 8.293 square units of support and fails all
32 routes. Neither candidate is published.

The diagnostic's three focused tests cover landing-only exclusions, active
flight/landing obstacles, unchanged inputs and explicit height extrapolation.
All 23 stair-navigation tests, Rust formatting and the main game build pass.

### Climbing clearance along the physical surface (2026-10-07)

The contact diagnostic now compares ground, surface and screen frames and
measures straight swept footprints and connected free-center components.
For a planar floor with gradient `g`, the surface frame uses the positive square
root of `I + g*gᵀ`. This preserves distances on the lifted 3D plane. The gable
candidate's upper inside anchors become supported at every tested rotation in
this frame, while the direct crossing still hits the roof slab. Configuration
space finds a supported detour between both inside anchors in every placement.
Screen clearance instead loses connectivity at 180 degrees, so it is not a
rotation-independent substitute. These polygon experiments alone do not prove
native actor traversal or rendered contact.

Physical ladder and wall queries now apply the inverse surface metric to the
actor's existing footprint. They preserve its width, depth and angles along the
plane; profile dimensions and the normal one-unit movement inset are unchanged.
Ordinary stairs retain their prior horizontal clearance. Sweeping the resulting
centrally symmetric footprint checks real support and live obstacles without
padding geometry. Closing-obstacle intersection and neighbour-query bounds use
the corresponding full footprint. Compound climbs use a conservative convex
envelope of their per-patch footprints, so a seam cannot under-check either
floor; narrow compound layouts still need broader verification.

The same `...gable-house-neighbour-placements-5s3m76` descriptors now complete
24/32 actor routes instead of 0/32. Both copies pass at 0, 37 and 180 degrees,
at both elevations. All eight 90-degree cases still fail at the upper door
handoff; the main inside-to-inside climb succeeds. Its first midpoint has
0.005525 square units of unsupported projected footprint and no slab overlap.
The unnormalized diagnostic puts it 0.181216 world-XY units from the nearest
free-center region; this is not a tolerance recommendation. No landing tolerance
was relaxed, and neither asset candidate is published.

Regression snapshot `surface-clearance-regression-ib6a1geu` passes all 84 existing
climbs: Derby 4, Leicester 22, Lincoln 8, Sherwood 10 and York 40. The other five
maps have no climbs in these descriptors. All 47 regular compiled-navigation
tests pass, as do moved-climb, compound-climb and copied-barrier actor tests and
all four changing-climb tests, including 72 mid-climb reopening checks. The 28
stair-navigation tests include metric length preservation, narrow-contact
rejection, real barriers, full-footprint crushing, neighbour bounds, compound
envelopes and invalid inputs. Eight diagnostic tests pass. Truly vertical and
overhanging surfaces remain outside the current height-field representation.
All 66 enabled map-compilation integration tests, Rust formatting and the main
game build also pass with the final clearance implementation.

### Wider climb placement checks and precise entrance binding (2026-10-07)

`check-courtyard-west-stair-neighbours.mjs` accepts an explicit `--rotations=`
list; its default remains 0/37/90/180 degrees. It retains each compiled descriptor
and editor scene before asserting connectivity, marking diagnostics incomplete
until every placement and disconnected-neighbour rejection has passed.

The gable authoring helper can slide the upper doorway along the equal-height
flight/roof seam by at most one local XY unit. It retains both planes, the slab,
and collision rules. The half-unit candidate `york-gable-roof-contact-iM9TlQ`
still passes only 24/32 routes (`...gable-house-neighbour-placements-CbUzcx`).
The one-unit candidate `york-gable-roof-contact-KiNzCZ` compiles 50 placements at
25 angles, two elevations and two copies, and rejects 300 disconnected-neighbour
cases (`...gable-house-neighbour-placements-9FaGRH`). Only 144/200 native routes
pass. Full York `saved-map-exports-tU05vA` passes 38/40 climbs: the lower gable
entrance cannot bind physical landing support and falls back to projected
navigation. The mesh review retains 596/602 sampled flight hits, maximum nearest
mesh height residual 0.872101 and maximum uncovered edge distance 0.039980.
The upper approach is on the separate roof and receives no support from the
climb asset mesh alone. These asset edits remain unpublished.

With surface clearance, the existing keep-annex candidate's original eight
placements (`lincoln-keep-annex-neighbour-placements-jlgO6X`) now pass all 32
native routes. The expanded sweep initially stops at 150 degrees; retained
failure `...keep-annex-neighbour-placements-er7ULT` shows that an exact supported
outside point projects to [1429.643952, 1299.807472], while [1430, 1300] lies
0.301092 units outside its receiving polygon. Increasing collision tolerances
would not address that loss of entrance precision.

Physical lift resolution now uses the exact projected world anchor. The loader
uses the existing physical navigation endpoints for runtime doors and selects
landing receivers using the physical outside point. Integer coordinates remain
compatibility identities; no duplicate endpoint data is needed in the export.
A compiler regression covers stairs, ladders and walls and verifies that genuinely
unsupported points do not gain physical traversal. A native regression verifies
receiver binding when the rounded approach lies outside a triangular roof, both
with and without explicit door endpoint fields. Floor contours, obstacle bounds,
and movement tolerances are unchanged. The expanded keep-annex compilation now
passes all 50 placements and rejects 200 missing/raised neighbours in
`lincoln-keep-annex-neighbour-placements-TjltD7`; native handoff verification
passes 196/200 routes and detects four receiver mismatches at 150 degrees, so
the asset is unpublished. Final exports without duplicate endpoint fields are
in `lincoln-keep-annex-neighbour-placements-qVQyOl`; their complete native rerun
confirms the same 196/200 result and the same four 150-degree handoff failures.

All ten saved scenes compile in `saved-map-exports-1tkbkL`. Fresh final exports
of Derby, Leicester, Lincoln, Sherwood and York in `saved-map-exports-sghWnY`
are identical after removing only the intermediate batch's redundant endpoint
fields; all 84 complete-character climb routes and all 192 stair routes pass.
All 139 compiler tests,
48 regular compiled-navigation tests and four changing-climb actor tests pass,
including 72 mid-climb barrier reopening checks. App typechecking and focused
lint pass. Pipeline typechecking remains blocked by unrelated state-delivery
test types. Two app export fixture comparisons (merged platform and sloped
terrain sockets) also fail with the unchanged compiler, verified by rerunning
those two cases against its HEAD version; they are not marked as passing here.
The final game build and Rust/editor formatting checks pass.

### Keep-annex approach extraction diagnosis (2026-10-07)

The earlier label "handoff failure" was too broad. A focused final-code rerun
in `lincoln-keep-annex-neighbour-placements-3BJQRg` records the active movement
command, orders, world position and whether the actor has entered the flight
when receiving-floor verification fails. Both unraised failures occur with
`crossed=false` and `MoveWaiting`, before the climb. The first actor is moved
to map [1433.7631, 1305.7365], 6.765310 units outside its roof and 7.209041 units
from the authored outside-to-middle approach. Its stale receiving plane then
evaluates height 453.96176 instead of the original outside height 350.94511.

The instruction's source extraction checks a projected move box and pushes it
away from intersecting movement lines. This differs from supported physical
climb extraction, which already checks world-coordinate clearance. Measuring
the exact bound landing and its live exclusions shows zero unsupported area
for the normal effective world-XY footprint at either copied lower entrance;
the equivalent projected rectangle lacks 19.897772 and 19.898947 square units
of support. The center's plane residuals are only 0.000123 and 0.000402 units.
This is a projected walking-footprint problem, not evidence for a larger seam
height tolerance or a changed roof contour.
The full source box must be considered separately: it lacks 0.203359 and
0.203470 square world-XY units at those two outside anchors, versus 41.027888
and 41.029084 projected units. Thus the normal movement inset's zero deficit
does not certify source authorization, and merely changing query coordinates
would not by itself prove this asset contact complete.

`audit-lift-support.py` now reports ordinary landing approach measurements in
both world and projected coordinates, with both the normal movement inset and
the full source box, alongside each outside anchor's height residual. It retains
holes and active obstacles, reports degenerate projections
explicitly, and never changes input geometry. These landing-only measurements
exclude the adjacent flight: the whole approach sweep is not certified merely
because its outside footprint fits. The four added tests cover projection
compression, live exclusions, edge-on floors and the distinct source box;
all twelve diagnostic tests
pass. Physical-space walking-source authorization and supported approach motion
remain required follow-up work. No navigation behavior or asset definition is
changed by this diagnostic commit.

### Physical ordinary-floor query infrastructure (2026-10-07)

The failing rotated roof is retained as a small geometry-only regression fixture,
`stair_navigation/fixtures/rotated-roof-approach.json`. It contains the compiled
bound receiver, its exclusions and the actor's physical outside point, with no
mission population or runtime data fallback. Independent configuration-space
erosion finds no supported projected centers anywhere on that receiver for
half-footprints [5,2], [6,3] or [6.5,3.5]. Moving the anchor to another projected
position cannot fix this case.

`PhysicalWalkingSurface` now shares the physical route solver with stairs while
retaining precise ordinary-floor contours and holes. Queries require endpoints
on the receiving plane and read an explicit current collision snapshot. Bound
landing snapshots retain the exact layer and sector filter and rebuild their
obstacles from live pathfinder state. Disconnected receiver fragments stay
separate. No lift is synthesized to represent an ordinary floor.

Source recovery first checks the full standing footprint. When correction is
needed, the result must fit the box expanded by half a unit, remain within a
caller-specified physical distance, and retain its actual receiving plane. The
recovery sweep cannot introduce unsupported space beyond the initial overlap;
off-floor centers and centers inside a solid are rejected. Ordinary routes use
the normal movement inset, keeping these distinct authorization rules explicit.

All 34 physical-navigation unit tests pass, including six ordinary-floor tests
covering compressed/edge-on projections, incorrect heights, full-box recovery,
holes, changing solids, narrow floors, malformed inputs and the compiled roof
fixture. The fixture recovers within three world units and retains a supported
walking route back to its doorway. Exact sweep subtraction resolves a tangent
contact that an independent relation query rejected despite having no unsupported
region; no clearance or height tolerance is enlarged.
All 48 regular compiled-navigation tests also pass; the 15 external-fixture
audits in that group remain explicitly ignored in this run. Rust formatting
and the game build pass.

This is query infrastructure, not completed actor integration. The movement
dispatcher, per-tick ordinary movement, dynamic-neighbour snapshots and receiving
ownership handoffs still need to use it together. The native keep-annex result
remains 196/200; no asset is published or runtime parity claimed by this change.

### Complete ordinary receiver binding (2026-10-07)

`LevelNavigationAssets::physical_walking` now retains complete ordinary receiving
floors independently of stair landing contacts. The loader groups coplanar
receivers by their exact motion identity, intersects them with the assigned
motion region and retains holes, disconnected components and live obstacle
state identities. Receiver geometry can retain pre-grid boundaries when their
rounded coverage belongs to that region. Visibility switches do not remove
walking floors. Binding does not synthesize a lift, clip against a stair flight,
or extrapolate a receiver plane across the rest of a multi-height motion region.

Three binding tests cover full-floor routing through close/reopen and serialized
binding restoration, disconnected support, holes and pre-grid boundaries. Two
loader tests cover an ordinary roof away from its doorway and a walkway map
without lifts. All 37 physical-navigation tests and 50 regular compiled-navigation
tests pass; the latter still explicitly ignores 15 external-fixture audits.
Rust formatting and the game build pass.

The focused copy of the earlier export in `walking-floor-bindings-sgzgks2f`
retains the expected two failures among four actor routes: ordinary movement
execution has not changed. Failure reports now include matching ordinary floors
and their full-box recovery queries. Both lower roof approaches bind correctly;
the native recovery queries succeed at physical distances 1.152761 and 1.152922
from their authored sources. This differs from the earlier trimmed stair-contact
fixture because ordinary walking uses the complete receiver. The recovery limit
in this diagnostic is three world units, not a new global movement rule.
Independent footprint sweeps from each recovered point back to its doorway
have zero unsupported world-XY area with the ordinary movement inset, retaining
all holes and active solids.

The next integration must use these floors for source authorization, route
dispatch, live-neighbour collision, per-tick walking and receiver handoffs
together. Motion imported only as projected contours still cannot reconstruct
an exactly edge-on ordinary floor; that case requires explicit world collision.
Neither the unchanged actor result nor these binding tests certify full parity.

### Ordinary physical walking execution (2026-10-07)

Point moves and gate approaches whose source and goal share a bound ordinary
floor now preserve world-space movement orders. Each step queries current solid
and neighbour collision, retains the receiving plane, and updates the actual
receiver and material. Neighbours on incompatible heights do not obstruct the
route. Connected ordinary stairs can support a footprint at their landing without
granting center ownership outside the floor; climb walls cannot support standing.

Source recovery stays within one actor-footprint radius plus half a unit. If the
expanded recovery box cannot fit a narrow roof, a position supported by the normal
movement inset can remain in place. Unsupported sources are still rejected.
Solid-boundary clearance includes a conservative coordinate-rounding guard;
native swept-footprint assertions remain unchanged. A same-XY endpoint with a
separately rounded, validated height no longer requires a nonexistent horizontal
waypoint. Height auditing evaluates the actual receiver at physical XY, avoiding
unstable inverse projection while retaining its 0.001-unit tolerance.

`physical-walking-placements-mmrlqy57` passes **200/200 complete-character routes**
across 25 angles, two elevations and two copies. This resolves the previous four
150-degree approach failures without widening roofs or moving door anchors.
The keep-annex candidate remains unpublished because its independent mesh review
still disagrees with the authored flight geometry. Passing these routes does not
certify rendered traversal or all navigation behavior.

`physical-walking-saved-w3g_sn36` also passes all **84 saved-map climb routes**:
Derby 4, Leicester 22, Lincoln 8, Sherwood 10 and York 40. These are exported
editor scenes tested through complete-character movement, not visual playthroughs.
The same exports pass all **192 complete-character stair routes**: Derby 28,
Leicester 16, Lincoln 16, Sherwood 2 and York 130. The game build and Rust
formatting checks pass.

The regular compiled-navigation suite passes 51 tests; 15 external-fixture tests
remain explicitly ignored in that invocation. All 38 physical-navigation tests,
two focused physical-walking regressions, 29 engine replay tests, 27 replay-format
tests and four changing-climb
tests pass, including 72 mid-climb reopening checks. Movement orders now retain
ordinary floor identities, requiring replay schema 67. Seek/line orders,
cross-floor routing, ordinary soft repulsion and exactly edge-on ordinary floors
remain outside this integration.

The next candidate audit, `physical-walking-york-ob7b61mv`, reruns the unchanged
one-unit gable/lean-to doorway candidate across all 200 placement routes. It
improves from 144 to **152 passing routes**: the 90-degree approach cases now
pass, while 105/120/135 and 285/300/315 degrees fail for both copies at both
elevations. A sampled 105-degree failure has a supported source, destination and
physical route but stalls inside the climbing sequence after entry. This needs
the active movement goal checked separately, as detailed below. The corresponding
full-scene candidate in `physical-walking-york-scene-u66e7h2r` retains its two
failed climbs (38/40 pass). These candidate assets remain unpublished; they are
distinct from the passing saved-map baseline above.

### Gable climb active-order diagnosis (2026-10-07)

The native stalled-route report now includes the active physical order's route
and animation row/frame/distance, facing, forecast and directional distance rows.
`york-climb-animation-jwt5fyuk` repeats the four failing 105-degree routes. The
inside-to-inside probe succeeds for the upward route, but its active order targets
the upper **middle** point and that route returns no path. All sixteen climbing
directions have three-unit distance entries; missing sprite movement data is
ruled out. A zero-distance sample alone was insufficient evidence about animation.

The contact-area diagnostic finds 0.001830 and 0.002487 square units of unsupported
upper-midpoint footprint for the two copies, with no overlapping solid. The
nearest free centers are about 0.154 units away. Flight and landing contact
precision need review before changing any geometry or tolerances. The previously
reported inside-anchor support must not be read as proof that the stalled active
order has a supported route. No candidate asset or runtime behavior changes in
this diagnostic step.

### Separate gable corner and binding failures (2026-10-07)

Comparing the full-precision exported flight and receiving roof, before runtime
rounding, confirms 0.001384 square units of unsupported midpoint footprint at
105 degrees. The corner shortfall is real; the existing rounding closure cannot
remove it. The asset-authoring helper now permits an explicit seam slide up to
two local units, retaining its zero default and both planes and collision shapes.
The candidate remains subject to placement and mesh review.

Stage `york-gable-roof-contact-jLHQwO` uses a 1.25-unit shift. Its focused exports
`...gable-house-neighbour-placements-faOe2I` compile six placements, reject 36
disconnected neighbours and pass 8/24 character routes: all 105-degree cases,
but neither 120 nor 135 degrees. The two-unit stage `york-gable-roof-contact-9D5MvH`
and exports `...gable-house-neighbour-placements-65EnfX` retain the same compilation
and disconnect results and pass 18/24 routes. All 105/135-degree routes pass;
six 120-degree routes still fail. No candidate asset is published.

The retained one-unit candidate's 120-degree trace
`york-climb-animation-jwt5fyuk/native-120-trace.log` identifies missing upper
landing bindings. The measured middle-to-support distances are 0.000594924 and
0.001190428 for the two copies, while both outside anchors lie on support. This
translation-sensitive binding failure is distinct from the real 105-degree roof
corner. The loader reconstructs precise motion contours using float32 receiver
plane coefficients; their precision and the support clipping remain to be
isolated before changing any acceptance tolerance. The focused candidate results
do not replace the earlier 152/200 full-sweep measurement.

### Receiver-plane fitting and complete gable placement sweep (2026-10-07)

The 120-degree comparison isolates coefficient rounding from receiver-anchor
rounding. With the same float32 anchors, fitting in double precision reduces
the first copy's middle-to-support gap from 0.000594979 to 0.000123650 units and
the second copy's from 0.001190442 to zero. Using the exported full-precision
anchors also gives zero for both. Physical walking and stair-landing bindings
now fit their existing receiver anchors in double precision and retain those
coefficients through collision reconstruction. Compatibility movement retains
its previous plane calculation. Degenerate or nonfinite physical planes report
an error; no clearance or height tolerance is enlarged.

The captured `rotated-landing-plane-precision.json` regression binds with this
fit, reproduces the disconnected landing with rounded coefficients, and rejects
a real 0.02-unit height gap. All 39 physical-navigation and 51 regular compiled-
navigation tests pass. The focused two-unit entrance candidate passes 24/24
routes in `precise-plane-york-ivffm_58`.

Fresh `york-central-lane-stone-gable-house-neighbour-placements-T0Cbsl` compiles
all 50 placements, rejects 300 disconnected-neighbour cases, and passes
**200/200 complete-character routes** across 25 angles, two copies and two
elevations. The candidate stage is `york-gable-roof-contact-9D5MvH`; the runtime
fit is general and reads no source-level fallback data. Its repeated mesh review
retains 596/602 flight hits, maximum nearest-mesh height residual 0.872101 and
maximum uncovered edge distance 0.039980. The upper approach belongs to the
separate roof; its absence from the climb asset's own mesh samples does not
measure the assembled roof. Rendered integration remains unverified.

The fresh candidate full scene `saved-map-exports-hr4SST` still passes **38/40**
climbs, with entrances 127 and 128 failing inside the gable lift. It remains
unpublished pending its connection to the surrounding map. Separately,
`precise-plane-saved-ie9dhna_` retains all **84 published-scene climb routes**
and **192 stair routes** across Derby, Leicester, Lincoln, Sherwood and York.
`precise-plane-keep-lkkqafqd` also retains all eight keep-annex 150-degree routes
across both copies and elevations. Rust formatting and the game build pass.

### Raised receiver hole boundaries and gable exclusion ownership (2026-10-07)

Raised receiving volumes previously checked climb approaches against rounded
navigation holes and rejected a midpoint on a hole boundary. Ordinary floor
landings already permit that seam contact. Raised receiver admission now uses
uniquely matched precise hole contours, falling back to the rounded contour when
no unique match exists. Only the midpoint may touch a hole boundary; outside
anchors and hole interiors remain blocked. The regression fails before the fix
and passes for ladders and walls at integer and fractional placements, while a
0.01-unit intrusion into the hole remains rejected. All 140 compiler tests, app
typechecking and focused lint pass.

Fresh York candidate export `saved-map-exports-MAyCih` is structurally identical
to `saved-map-exports-hr4SST`; this admission fix does not resolve its two climb
failures. The lower outside and midpoint both lie on raised receiver 87 at
height 90.00101 and within its underlying motion boundary. The midpoint is inside
motion area 0's permanent obstacle 18 on layer 19, whose triangle is
[[1943,1485],[1753,1663],[1683,1639]]. Physical traversal is therefore omitted
during compilation, rather than lost at runtime landing binding.

Asset-local blocker inspection identifies
`york-central-lane-rear-gable-house/building-168-ground-blocker-37-0-0` and
`york-west-town-raised-terrain/building-086-ground-blocker-368-4-0` at that point.
Both carry movement contour `york-terrain/ground-section-0/exclusion-4`. The
raised terrain supplies a physical receiver at height 90.00101 over navigation
height zero. The next authoring review must resolve the actual collision ownership
and any residual exclusions tied to surrounding buildings; admitting an interior
point or enlarging a tolerance would conceal that issue. No asset metadata is
changed or published in this step.

### Physical terrain ownership experiment and collinear obstacle partitions (2026-10-07)

World-space inspection of the gable's lower midpoint confirms that the rear
gable-house blocker is at height zero and does not contain the physical XY
position at height 90.00101. The raised terrain does contain it, and its top
supports that height. No placed solid other than the supporting terrain contains
that physical point. This separates the stale projected exclusion from actual
physical contact.

`refinement/stage-raised-terrain-surface.mjs` creates an unpublished authoring
candidate from pinned assets: an explicit flat deck replaces the terrain's
projection-only receiver and inherited exclusions. Intersecting neighbouring
parts supply placed solid collision, except explicitly reviewed movement-solid
selections and state-controlled parts. It does not publish or change saved scenes.
Stage `raised-terrain-surface-2t5N1c` removes 19 terrain-owned exclusions and
selects 255 neighbouring parts; generated receiving areas remain independent
where collision disconnects the deck. Mesh, material and moved-placement review
remain required before any publication.

This experiment exposed a compiler failure in obstacle island triangulation:
a bridge triangle had area approximately 1.3e-12 and aborted the entire export.
`collinear-obstacle-islands.json` preserves the failing geometry. Partitioning
now calculates signed area relative to a local origin and omits degenerate
triangulated ears, while retaining the existing whole-region coverage check and
rejecting degenerate authored outer rings. The new test fails before the fix,
passes afterwards and checks collision difference below 1e-8. All 163 focused
compiler, boundary, partition and ground-recovery tests pass, alongside app
typechecking, focused lint, formatting and the game build.

Fresh candidate `saved-map-exports-cpF6Ar` exports successfully and restores the
gable's physical navigation definition. It is not a publication candidate:
omission warnings rise from 8 to 45 and other entrance/receiver contacts need
review. The completed native audit still passes only 38/40 climb routes. The
gable now has physical navigation (sector 614), but its actor footprint lacks
support at the lower handoff; point-level admission alone is insufficient. The
published baseline and the earlier 200/200 isolated placement result remain
unchanged. This broad ownership experiment is diagnostic, not a parity success.

### Fractional collision edges across independent solids (2026-10-07)

The physical-deck candidate's lower gable handoff lacked 0.001499 square game
units of footprint support. Closing representation-scale cracks did not remove
the wedge: a nearby precise obstacle still contained integer-grid edges. Edge
recovery had rejected the entire obstacle when its rounded outline intersected
more than one disconnected source solid.

`restoreObstacleBoundary` now considers edges from all intersecting source
solids independently. Only a uniquely identified edge can be restored; competing
edges retain their rounded geometry. The result is unioned with all previously
represented source collision, and must still round to exactly the same integer
obstacle. This does not merge or discard asset collision ownership or increase
actor clearance tolerances.

A regression adds a separate subpixel collision island to an existing landing
fixture. It fails before the change and passes afterwards, including quarter
turns with translated coordinates. It checks that the separate island remains
solid, the false landing strip disappears, the integer contour is unchanged and
ambiguous duplicate sources cannot authorize a cut. All 159 focused compiler,
boundary and precise-contour tests pass; app typechecking, focused lint,
formatting and the game build pass.

Fresh export `saved-map-exports-eXYtfj` from stage
`raised-terrain-surface-2t5N1c` passes **40/40 complete-character climb routes**
in the native engine, including both gable directions. The actor audit finished
successfully in 101.02 seconds. This supersedes the candidate's 38/40 result
above. The stage remains unpublished because its 45 omission warnings and other
feature coverage still need review. Fresh published-scene export
`saved-map-exports-fIgcrk` is structurally identical to the previously verified
published York descriptor in `precise-plane-saved-ie9dhna_`; its fresh native
audit also passes all 40 climb routes in 85.35 seconds.

The candidate's full native stair audit completed in 346.88 seconds with
**126/130 routes passing**. Both directions fail for
`york-east-riverside-curtain-wall/building-261-lift` (sector 616) and
`york-market-southwest-connecting-stairs/building-474-lift` (sector 619).
The former binds only its upper landing; its lower midpoint lacks approximately
20 square game units of support. The latter binds both landings, but its lower
approach center extends approximately 0.0869 units beyond receiving support.
These are larger contact problems, not grounds to increase numeric tolerances.
The next authoring review must compare newly enabled part-solid collision with
the corrected stair floors and their existing local clearances. The candidate
remains unpublished; the structurally unchanged published descriptor retains
its previously verified 130/130 stair routes.

### Published stair flight movement volumes (2026-10-07)

The four terrain-experiment stair failures above came from part-solid footprints
that no longer matched the corrected authored flight floors. The authoring helper
`refinement/stage-stair-movement-volumes.mjs` constructs an explicit movement
volume from each planar floor and the part's bottom plane. It replaces only that
flight's movement-solid reference, leaving visual and sight geometry intact.
The helper verifies descriptor pins, validates the gameplay definition and stages
the result for review. `--standalone` retains the published asset's unrelated
collision choices rather than inheriting a larger experiment's extra solids.

Broad candidate `saved-map-exports-c4boWH` now passes **130/130 native stair
routes**, **40/40 climbs** and all **six geometry control apply/reset checks**.
It remains unpublished with 45 omission warnings. An initial stair-only subset
still enabled unrelated wall solids and introduced two new feature omissions;
that subset was rejected. Final stage `stair-flight-volumes-only-avx6SQ`, also
reproduced by the helper in `stair-movement-volumes-sDnYnL`, preserves those
unrelated collision choices. Its export `saved-map-exports-zcJVpZ` retains the
baseline eight omissions and 28 doors, with no new feature warnings. Native
audits pass **130/130 stairs**, **40/40 climbs** and all **six controls**.

Independent placement batches
`york-east-riverside-curtain-wall-neighbour-placements-aq2Ckf` and
`york-market-southwest-connecting-stairs-neighbour-placements-02EmgP` each test
eight exports, sixteen copied placements and 48 disconnected-neighbour
rejections. Each passes **32/32 complete-character native stair routes**. The
neighbour fixture also supports terrain assets that own a physical floor:
removing that terrain must still reject the connection, while removing unrelated
editor ground no longer needs to break an independently supported landing.

The two flight-volume definitions are now in the main local library, with its
generated catalog refreshed and the saved York descriptor pins updated. Backup
and publication hashes are in `york-stair-volume-publication-5jvncsnw`. All ten
saved scenes reopen. Fresh published export `saved-map-exports-4SB13a` is
structurally identical to the native-verified candidate apart from draft-warning
wording. Four fresh published placement exports at two elevations also match
their native-verified candidates apart from warnings, and reject all 24 checked
disconnected-neighbour cases. Rendered integration warnings remain explicit; the gable, lean-to and
broader terrain experiment remain unpublished.

The native stair audit accepts optional `ROBIN_LIFT_AUDIT_SECTORS=616,619` for
focused diagnosis and records the filter in its report. Unfiltered full audits
above still test every stair. Focused lint/format checks, Cargo formatting and
the game build pass.

### Physical doorsteps on independently authored terrain (2026-10-07)

The physical-terrain experiment's omitted west-town lane entrances were placed
on sloped receiver volumes that supplied rendering height but no explicit
walking floor. Merely enabling those volumes as movement solids blocked the
projected ground beneath them. `refinement/stage-receiver-floors.mjs` explicitly
promotes selected receiver tops to asset-local walking floors and exposes their
outer edges as navigation sockets. It checks every staged descriptor pin,
requires selected solid support, refuses unreviewed material partitions and
retains the fitted top plane. No runtime sector IDs or saved-layout fallback
are introduced.

Staging uses `terrain-volume-stage-rebase-PRbGPE`, which updates only the two
published stair pins in the preceding experiment while preserving its explicit
experimental gameplay. Final stage `receiver-floors-kEzmuf` adds the
`building-601-receiver` and `building-602-receiver` floors to
`york-west-town-lane-stone-tower` and `york-west-town-lane-narrow-timber-house`.
The authored fourth height differed from each volume's fitted plane by at most
0.000004599 and 0.000010356 units respectively. Retaining those rounded heights
made the initial placement batch fail planarity after rotation to 180 degrees.
Evaluating all floor vertices on the already validated volume plane fixes that
authoring inconsistency without increasing compiler tolerances.

`refinement/check-receiver-floor-placements.mjs` places two copies of each house
over newly authored flat terrain at 0, 37, 90 and 180 degrees and two elevations.
Final batch `receiver-floor-placements-bidY5j` produces eight exports and
**32 connected doorstep placements**. Each doorstep must have exactly one
terrain socket match; removing terrain or raising it 20 units rejects all
**64 checked connections**. Building entrances remain defined on their own
doorstep even when the terrain connection is absent. These are compiler checks,
not proof of complete actor routes.

The native sampled receiver audit passes **32 directed crossings** in that
batch, covering the 0- and 37-degree placements at both elevations. It reports
zero eligible receiver pairs at 90 and 180 degrees. Those orientations therefore
remain unverified by actor movement despite the test's successful exit status.
The report is `actor-receiver-crossing-report.json` in the placement batch.

Full candidate `saved-map-exports-49YG0O` restores two building records (68 to
70), the two formerly omitted receiver bindings and the raised terrain's
`light-14 receiver 1` binding. Omission warnings decrease from **45 to 40**;
door/receiver/jump/light omissions decrease from **20 to 15** with no newly
omitted feature. The full native geometry-control audit loads this candidate
and successfully applies/resets all **six controls**. Complete door approach
routes, the unsampled orientations and rendered contacts remain outstanding.
The helper and placement verifier pass focused lint and formatting. Neither
doorstep nor the broader terrain candidate is published by this experiment.

### Dispatched doorstep approaches and destination receiver selection (2026-10-07)

The placement verifier now supplies an explicit physical approach probe for
each entrance. It starts 24 world-XY units outside the doorstep's lower edge on
newly authored terrain, retaining the compiled doorway endpoint. Extending the
projected doorway direction is unsuitable because a turned slope can compress
or reverse that direction. Each probe must identify exactly one exported
building entrance.

Batch `receiver-floor-placements-jzIo86` retains all eight successful exports,
32 copied placements and 64 disconnected-terrain checks. The new ignored native
audit `exported_building_approaches_support_dispatched_actor_routes` dispatches
ordinary Move requests, processes path requests and ticks actors in both
directions for every entrance. It records outcomes even when routes fail;
there is no geometry-eligibility filter that can silently skip an orientation.
Its scope is the standard native walking fixture with a 6-by-3 half-diagonal,
not complete character sprites or building transition animations.

The initial audit completed with **32/64 routes passing**. All approaches onto
the doorsteps kept the ground receiver after entering the slope; approaches away
passed. Same-floor physical dispatch had accepted a projected destination by
unprojecting it onto the current floor, even when the actual receiving query
selected a raised neighbour. The destination could therefore appear supported
on the wrong plane. Dispatch now requires an implicit destination's receiver
to belong to the selected physical floor; other moves retain their receiver
transition path. Explicit world endpoints retain their existing height checks.

After the fix, the same batch passes **64/64 directed routes**, covering both
assets and copies at 0, 37, 90 and 180 degrees and both elevations. The report
`actor-building-approach-report-before-receiver-selection.json` retains the
completed failing baseline; `actor-building-approach-report.json` contains the
completed passing audit. A regular regression covers an overlapping flat floor
and sloped receiver, checking both movement directions. The compiled-navigation
suite passes **52 tests**, with 16 explicitly ignored external-data audits.
Focused verifier lint/format checks, Cargo formatting and the game build pass. These results
supersede the earlier sampler's missing 90/180-degree approach coverage, but do
not complete world-space cross-floor routing or certify building entry/exit and
rendering. The doorstep/terrain asset candidate remains unpublished.

### Explicit destination layers in physical walking dispatch (2026-10-07)

A regression with two valid overlapping screen footprints at different heights
showed that the physical-walking shortcut also bypassed normal routing for an
explicit destination on another layer. Before the fix, the upper-layer request
returned `Success` with a lower-floor physical walking order. The shortcut now
checks a specified destination's layer, public sector and available runtime
sector identity before resolving a screen point on the current floor. An
unspecified sector retains the existing local-destination semantics. Explicit
gate world endpoints continue through their existing floor checks, and deferred
door-handoff routing retains its own direct-movement rules.

`physical_walking_shortcut_respects_the_requested_destination_layer` passes for
a local explicit goal, an upper-layer goal that remains queued for normal
routing, and a local goal without a specified sector. This verifies dispatch
ownership, not complete traversal between disconnected floors. The full
compiled-navigation group passes **53 tests** with 16 external-data audits
ignored. All **22 path-request timing/direct-movement tests** pass, including
the deferred door-handoff regression; the direct binary invocation requires the
standard `RUST_MIN_STACK=33554432` setting (the initial default-stack invocation
aborted in a direct-movement test). The existing doorstep batch
`receiver-floor-placements-jzIo86` still passes **64/64 directed approaches**.
Cargo formatting and the game build pass. General physical cross-floor routing, complete building
entry/exit animations and rendered integration remain unfinished.

### Complete-sprite doorstep approaches and building exits (2026-10-08)

The `receiver-floor-placements-jzIo86` batch now has separate complete-sprite
approach and building round-trip audits. Both use the complete RobinTown sprite
with the existing 6-by-3 movement half-diagonal. Virtual building interiors have
no receiving floor; receiver checks resume after the exit passage finishes.
The round-trip audit requires actual entry, exit and completed passage orders,
so merely reaching the outside door point cannot pass it.

All **64/64 complete-sprite approaches** pass. Initially 40 were reported short
by approximately 0.01 units despite completed orders. End-transition insertion
reserves that distance for a zero-distance animation, which then plays without
displacement. The audit permits this residual plus bounded coordinate rounding
only after the matching zero-distance stop animation and order chain finish;
receiver and height checks still run every tick.

Building round trips initially passed **4/32**. Of the 28 failures, 22 completed
the exit on an incorrect receiving floor and six failed before entering.
Binding the outside receiver at a compiled building's midpoint prevents the
later elevation crossing from toggling the preselected doorstep receiver back
to underlying terrain. Compatibility exits without physical walking bindings
retain their destination-point receiver rule. After the correction **26/32**
round trips pass, with all 22 receiver mismatches resolved. The six remaining
failures are stone-tower approaches: copy 1 at 37 degrees and both copies at
90 degrees, each at heights 0 and 40. No entry/exit or rendering parity claim
is made for those placements.

The three report files distinguish synthetic approaches, complete-sprite
approaches and complete-sprite round trips. Failed baselines are retained as
`actor-building-round-trip-before-handoff.json` and
`actor-building-sprite-approach-before-stop-contract.json`. Current round-trip
output has `audit_finished: true` and `complete: false`; the unpublished asset
candidate remains unpublished.

The focused `compiled_building_exit_binds_the_floor_at_its_midpoint` regression
checks both physical-floor and compatibility receiver selection against two
distinct receiving planes. It passes after correcting the test fixture's
interior sector identity. All 53 existing compiled-navigation tests and all
33 door-passing tests pass; 18 external-data navigation audits remain ignored
in the ordinary group. Formatting and the game build pass.

### Generated receiving planes survive clipping and copied placement (2026-10-08)

The complete-sprite building audit distinguished source authorization from
direct path dispatch. At 37 degrees, one stone-tower copy loaded an incorrect
receiving slope because clipping introduced nearly collinear first vertices.
The compiler now writes explicit `projection_plane` anchors for generated
receivers using both projected bounding spans and the existing height plane.
The anchors do not enlarge receiving coverage. Explicit asset anchors and
physical volume plane semantics remain unchanged.

Fresh batch `receiver-floor-placements-pXwJDW` retains 32 copied, rotated and
elevated placements and rejects all 64 disconnected-terrain cases. Synthetic
and complete-sprite direct approaches each pass **64/64**. Complete building
round trips improve from 26/32 to **28/32**, resolving both 37-degree failures.
The four remaining failures occur before entry at 90 degrees, for both copies
and elevations. Their diagnostic reports now include the ordinary floor,
collision, source support and bounded recovery result. The footprint extends
past the small doorstep toward adjoining terrain; ordinary neighbouring-floor
support still needs integration. Direct path-dispatch success does not cover
this movement-command authorization stage.

Full York candidate `saved-map-exports-aZY6Fi` retains 70 buildings and exactly
the earlier warning set: 1,176 warnings, including 40 omissions and 15 omitted
door/receiver/jump/light features. All **130 complete-sprite stair routes**,
**40 complete-sprite climb routes** and **six geometry-control apply/reset
checks** pass with the new receiving planes. The doorstep and physical-terrain
assets remain unpublished.

Focused compiler/plane/stair checks pass **151 tests**. Native fixture updates
add 111 plane definitions across 28 files; a recursive comparison verifies
that every pre-existing field and array member is unchanged. Editor export
comparisons retain only the two previously recorded merged-platform and
sloped-terrain-socket geometry mismatches. The new anchor fields do not hide
those differences. App typechecking, focused lint, formatting and the game build
pass. The full native suite initially passed 4,384 tests and exposed 19 fixture
helpers that changed receiver geometry without updating its new anchors.
Those helpers now transform or redefine the anchors with their polygons;
all **54 navigation tests** and the receiver-state regression then pass.
The 32 external-data tests ignored by the full suite are not counted as passes.

### Ordinary floors support feet across precise receiving seams (2026-10-08)

The remaining stone-tower failures combined two issues: the standing footprint
crossed the low doorstep edge, and integer clipping trimmed that edge to a
different height. Ordinary receiving contours now retain their authored
pre-grid precision even without a lift. Native floor binding precomputes
nonzero shared-edge connections only within the same layer and motion sector,
requiring compatible heights at both contact endpoints. Incompatible raised
contacts cannot provide foot support through the surrounding terrain. Query-time
support subtracts the neighbour's current obstacle states; the character center
still belongs to its current floor. This is not full world-space cross-floor routing.

Fresh `receiver-floor-placements-wG7jw2` passes **32/32 complete-sprite building
entry/exit round trips**, both **64/64 direct approach** suites, and all **64
disconnected/raised-terrain rejections**. These are two copies of two assets on
independently authored terrain, at four rotations and two elevations. All prior
four 90-degree failures are resolved without relaxing height matching.

New regressions cover edge versus corner contact, real gaps, wrong heights,
wrong layers/sectors, holes, live blocker apply/reset, retained floor ownership,
translated coordinates and ordinary fractional receiver contours without lifts.
The focused compiler/plane/stair suite passes **152 tests**; the **54 existing
native navigation tests**, the new movement-command seam regression and nine
walking-binding tests pass. The broader editor
fixture comparisons still have the two previously recorded merged-platform and
sloped-terrain-socket mismatches. Full candidate `saved-map-exports-SlmjWZ` has
the identical 1,176-warning set, including 40 omissions and 15 feature omissions.
Its six geometry controls apply and reset, and all **130 complete-sprite stair
routes** and **40 complete-sprite climb routes** pass. A final emitter cleanup
avoids redundant precision fields and uses fixed-point contour comparison to handle coincident
edges. `receiver-floor-placements-pUQGfB` matches the verified eight placement
descriptors exactly. Full candidate `saved-map-exports-Ui295T` differs from the
native-audited input only by omitted `precise_polygon` fields equal to their
integer polygons, verified recursively. App typechecking, focused lint, formatting
and the game build pass. The doorstep/terrain authoring experiments remain
unpublished.

`refinement/audit-gameplay-coverage.mjs` now provides a repeatable current-library
inventory, checking descriptor hashes and identifying actual saved placements
separately from unused library references. `gameplay-coverage-DfT8Rc` confirms
38 missing definitions among 1,297 assets, including four placed in Croisement03.
Only 32 missing definitions have all part names represented in gameplay-bearing
assets of the same map; the previous checklist count of 36 combined different
library revisions. The four newer Croisement03 assets and two old ground assets
have no component-owner match. Matching names remain authoring leads, not proof
that gameplay can be copied without coordinate/control review.

The fern construction records identify native masks 35 and 76. Mask 35 includes
character, projectile and view behavior, with distinct character/projectile
boundaries. The new fern mesh has no gameplay definition, and merely declaring
it scenery would not establish equivalent masking. Restoring its asset-local
mask behavior requires review of the separated fern/bark ownership.

`stage-fern-gameplay.mjs` now constructs unpublished definitions directly from
the two fern GLBs, including physical alpha cutouts. Initial extraction rejected
UVs below zero on clamp-to-edge materials (minimum -0.083866 for fern 35 and
-0.098072 for fern 76). Mask recovery now honors clamped sampler axes by extending
edge texel coverage, without clamping triangle vertices and distorting interior
UV interpolation. Unclamped out-of-range axes remain rejected. Thirty focused
alpha/mesh/mask tests pass, including all four clamped edges, constant exterior
UVs, mesh sampler propagation and retained alpha ownership semantics.

Candidate `fern-gameplay-zw6Geo` passes twenty exports across five rotations,
two terrain elevations and fractional placement. Each retains character,
projectile and view flags, nonempty coverage with transparent pixels, and
boundary ranges covering occupied pixel centers. Twenty mismatched terrain
height cases reject unresolved receiving anchors. Native construction loads all
twenty descriptors and verifies mask bitmap decoding and receiving-layer
registration (`/tmp/fern-zw6Geo-native.txt`). Ferns add no movement collision.

The mask front boundaries are explicitly authored closed canopy footprints on
the local ground plane, not recovered open polylines. Their rendered contact
with characters/projectiles still needs review. The candidates remain
unpublished and the missing-definition inventory is unchanged. The two staged
definitions total approximately 6.1 MiB before compression; the coverage is
derived from the assets, with no level-data dependency in authoring or export.

Browser diagnostic `app/tests/fern-mask-review.html` composites the pinned full
GLBs with Robin's actual idle sprite and exported point-mask rules. It copies
the baked background at every applied mask pixel, including transparent leaf
gaps; this is diagnostic compositing, not the engine's bounding-box mask query
or complete native renderer. Staging now saves each editable scene alongside
the descriptor to reproduce the exact placement.

The comparison exposed missing one-sided coverage semantics. With nearest
sampling, the two-sided candidates masked 331 transparent pixels for fern 35
and 437 for fern 76 at zero rotation; rotated cases masked 36–146. Explicit
`cullBackfaces` asset metadata now filters placed triangle winding during mask
rasterization. Existing definitions remain two-sided unless authored otherwise.
The fern author checks all materials are one-sided before setting it.

Candidate `fern-gameplay-At1ME3` retains all twenty construction and wrong-height
checks, and all twenty native descriptor/bitmap/layer checks pass. The 32 browser
composites show zero transparent-pixel mismatches for either canonical fern;
quarter-turn views still have 6–17 mismatches, requiring rasterization review.
The artifact is `fern-gameplay-At1ME3/compositing/after.png`. Both ferns remain
unpublished. All 144 affected mask/compiler tests, both typechecks, focused lint
and formatting pass. This does not establish projectile or complete rendered
in-game parity.

The expanded two-way fern diagnostic also counts rendered leaves absent from
mask coverage. At rotations 90/180/270, fern 35 has excess/missing pixel counts
17/10, 6/5 and 13/12; fern 76 has 9/9, 8/9 and 11/9. Both canonical views match
exactly. Every rotated view includes discrepancies without opposite coverage
within an eight-neighbour one-pixel radius. The complete diagnostic deliberately
fails and retains all 32 panels for review (`compositing/coverage-complete.log`).
No rasterization tolerance was relaxed to hide these discrepancies.

Full Croisement03 with the staged fern definitions initially failed because an
asset repeated three advisory draft issues. Best-effort compilation now keeps
each distinct warning once without mutating the descriptor. Strict validation
still rejects duplicate warnings, and empty issues remain invalid in both modes.
All 16 draft compilation tests pass, including unchanged geometry and source
metadata checks. Export `saved-map-exports-fGGgEs` then passes native geometry
construction (11 motion areas, 106 sight obstacles, 15 doors, 10 jump pairs,
5,016 grid blocks, 65 elevation boundaries and three masks) and all nine
recovered state apply/reset checks. This does not publish the fern candidates
or resolve the missing fallen-log and bridge definitions.

The browser diagnostic now separately renders the authored alpha-clipped
triangles on the GPU. At 90/180/270 degrees, CPU versus clipped-GPU differences
are 23/9/25 pixels for fern 35 and 18/17/18 for fern 76. Clipped-GPU versus
textured-GPU differences are smaller: 10/2/4 and 8/2/8 respectively. Both
canonical views agree in all three paths. This localizes most discrepancies to
rasterization, with a smaller texture/clipped-geometry discrepancy still open.
The reproducible subpixel sweep (`compositing/coverage-subpixel.log`) reduces
CPU/clipped-GPU differences to 3/4/7 and 7/3/5 pixels using a 1/16-pixel grid;
1/256 and 1/65536 grids do not give comparable reductions. This is evidence from
one software renderer, not a portable precision contract. Production sampling
is unchanged. A subsequent edge-rule experiment did not produce fixture results
before the browser observation failed; its unverified code was removed.

`stage-croisement-bridge-gameplay.mjs` constructs an unpublished Croisement03
timber-bridge deck definition from the published GLB alone. It checks descriptor
hashes and mesh frames, selects the two large upward-facing deck triangles, and
stores their local heights and two short end-edge connections. The local ground
contact is 51.917254873 game-height units; placing the asset at terrain height
therefore aligns its deck rather than its pier foot. The mesh's small export
roundoff is retained, with an explicit 0.001-unit end-connection tolerance.

Candidate `croisement-bridge-gameplay-4Ui7JF` passes ten strict exports at
rotations 0/37/90/180/270 and elevations 0/40. Every matching placement shares
one navigation region with terrain; all twenty terrain offsets of minus/plus
one unit remain disconnected. All ten native descriptor construction checks
pass (one navigation area per scene). These fixtures have continuous terrain:
they prove end-region matching, not actor crossings over a gap. Rail/pier solids,
gap crossings, character masking and publication remain unfinished. No source
level data is loaded by the authoring script or compiler.

The bridge staging script now also builds two separate terrain banks from its
transformed end edges, with no ground under the deck. Removing the bridge leaves
two disconnected navigation areas. Deck-only candidate
`croisement-bridge-gameplay-2Uriu3` passes all 40 directed native actor crossings:
20 over continuous terrain and 20 across the gap at five rotations and two
elevations. Twenty wrong-height landing checks still pass.

The explicit `--solid-deck` candidate derives bottom heights from the mesh's
two underside triangles and adds physical wooden slabs with upright headroom.
Candidate `croisement-bridge-gameplay-3Y2wBi` passes 240 native sight/projectile
checks through, above and below the slabs. Actor testing fails at the 270-degree
gap seam: position [587.1412, 458.8348] has no current receiver while the native
query finds a deck receiver. Binding the surfaces to those volumes also fails
the same route (`g5vpPr`). Fitting horizontal planes was investigated but caused
coplanar receiver conflicts and then a quantized collision sliver; that
experiment was removed. No collision padding, actor-test relaxation or library
publication was performed. The solid candidate remains explicit and unverified;
rail/pier collision and rendered integration are still required.

Further inspection shows the solid cuts were separating one bank's receiving
area, rather than an isolated renderer issue. Candidate `--solid-deck
--flat-deck` fits the nominally horizontal deck and underside independently;
the maximum height change is 0.0000151033 game units. Wood has explicit material
priority over coplanar ground. A compiler bug previously prevented this case:
terrain fragments produced by subtracting placed floors were subjected to a
half-pixel area threshold before navigation union. The compiler now preserves
these fractional polygons and holes at its existing continuous-geometry
threshold until final navigation processing. A regression test leaves a
0.02-square-unit terrain corner and verifies the final full square is retained.

Candidate `croisement-bridge-gameplay-axLNH0` passes all 40 native directed actor
crossings and 240 sight/projectile probes, with twenty wrong-height rejections.
All 146 affected compiler/navigation/terrain tests pass. The application
typecheck passes; the pipeline typecheck encountered unrelated concurrent
`state-delivery.test.ts` errors. This resolves the tested solid-deck crossing
failure, not rail/pier collision or rendered actor integration. Publication is
still pending those requirements.

`author-croisement-bridge-structure.py` now derives the five non-deck timber
components from the asset mesh. Four vertical extrusions are fitted within
0.000084652 scene units, keeping their front-profile openings. The brace retains
its convex mesh vertices. Adjacent cap triangles merge only when their union is
convex; per-component volume error stays below 0.0031%. Merging caps and coplanar
hull faces reduces the runtime representation from 2,835 to 526 solids,
including the deck. Partition volume conservation is checked independently.

Seventy-two ray probes use triangle intersections against the entire asset mesh
as their oracle, including 32 clear openings and 40 wood hits. With the deck
probes, five rotations and two elevations, candidate
`croisement-bridge-gameplay-20Dvs0` passes all 3,120 native sight/projectile
queries. Independent actor runs (`actor-case-report.json`) pass both directions
in 18 of 20 scenes. The 180-degree gap scenes at elevations zero and forty still
fail receiver tracking; all continuous-terrain cases pass. This is not full
traversal certification or publication.

The preceding structured candidate exposed a distinct 90-degree disconnection:
terrain rounded its shared edge before precise asset floors and rail collision
were assembled. Terrain surfaces now preserve fractional coordinates through
assembly. The minimized regression contains one rail solid, two deck triangles
and two terrain banks, with no model or level-data loading. Previously it
compiled into two navigation regions; now it compiles into one, while removing
the bridge still leaves two. All 146 existing compiler/navigation/terrain tests
and the additional regression pass. Application typechecking passes; pipeline
typechecking still encounters the separately modified state-delivery tests.
Full rendered occlusion and the remaining 180-degree receiver failure are open.

The 180-degree failure was an artificial internal seam between adjacent
generated ground receivers with identical material and receiving plane.
`partitionProjectionMaterials` now unions adjacent equivalent generated supports
before subtracting their coverage. It preserves physical receiver identities,
different materials/planes and ordering between non-equivalent supports. A
focused regression checks a fractional quadrilateral made from two triangles,
unchanged input metadata, separate material boundaries and physical identities.
All 156 affected terrain, compiler, navigation and partition tests pass, along
with application typechecking and focused lint/format checks.

Structured candidate `croisement-bridge-gameplay-59LmI4` passes all forty native
directed actor crossings and 3,120 sight/projectile queries, retaining all twenty
wrong-height landing rejections. Exporting the complete saved Croisement03 with
the bridge candidate (`saved-map-exports-6W8tQm`) passes native construction:
12 areas, 633 sight obstacles, 15 doors, ten jump pairs, 5,434 grid blocks and
68 elevation boundaries. All nine state apply/reset checks pass. This verifies
loading and control construction, not original-scene bridge crossings or rendered
actor occlusion. The bridge remains unpublished pending that integration review.

`check-saved-bridge-contact.mjs` now compiles the saved Croisement03 scene from
its pinned editor assets, transforms the bridge's end sockets and emits both
contact diagnostics and a native actor-crossing manifest. Unlike the isolated
fixtures, this scene uses `croisement03-terrain` as a background asset rather
than an authored terrain grid. Its ground still includes the old bridge's
walking footprint.

The asset's nominal deck plane is scene Z=0; averaging exported vertex heights
had placed it 0.000146346 game units below that plane at its saved placement.
Staging now converts that authored contact into asset-local coordinates, with
an explicit 0.001-unit maximum fit error, instead of retaining the export drift.
`croisement-bridge-gameplay-pT76XG` still passes all forty isolated actor crossings
and 3,120 ray checks. Its saved-scene contacts now have exactly zero height.
This authoring correction does not introduce level-data access in the compiler.

Saved-scene candidate `saved-bridge-contact-GKNHyl` exposes the remaining issue:
inside both bridge ends, layer zero has two overlapping areas (bridge sector
zero and background sector one); outside, only background sector one remains.
The native crossing fails on entry at [1024.1682, 743.276]: it acquires bridge
receiver 632 while the actor remains in background sector one. This is an
integration failure, not a successful original-scene roundtrip. Floor ownership
and connections to background terrain need correction; moving the bridge must
not leave its old crossing embedded in the terrain asset. Publication and
rendered occlusion verification remain pending.

`stage-bridge-ground-ownership.mjs` now authors a candidate terrain hole from
the placed asset's own deck geometry and two asset-local open end passages.
Candidate `bridge-ground-ownership-5HJmCM` checks five interior centerline
samples: each has exactly one navigation owner when placed and none when the
bridge is removed. Each passage has unique endpoints in distinct ground/deck
areas. Native construction passes for both exports: placed has 12 areas, 633
sight obstacles and 17 doors; removed has 11 areas, 106 sight obstacles and 15
doors. Both construct 10 jump pairs and 5,016 grid blocks.

These are ownership and construction checks only. The existing direct-walking
endpoint helper keeps a fixed sector and cannot verify a route through the two
open passages. Native pathfinding/actor traversal, alternate background
placements, rendered occlusion and publication remain pending. The terrain hole
is an asset-authoring correction; compilation still reads only editor assets.

The two-passage candidate is superseded: both banks share a sector, and the
native gate planner returns an empty route between identical sector identities.
An artificial forced gate route would not demonstrate a normal map click.
Asset surfaces now explicitly opt into ground attachment with
`acceptsNavigationJoins`. Matching exterior sockets join placed floors directly
to current ground geometry; coplanar floors own their coverage. Changed or
joined preserved ground is rebuilt as generated navigation. Ordinary boundaries
without exterior sockets remain separate.

Candidate `bridge-ground-ownership-zS6rzH` and contact export
`saved-bridge-contact-q9AzRp` now pass both native actor crossings across the
saved scene's bridge, including ground/deck receiver handoffs. Native loading
constructs 11 areas, 633 sight obstacles and 15 doors with the bridge; removal
constructs 13 areas, 106 sight obstacles and 15 doors. Both retain 10 jump pairs
and 5,016 grid blocks and pass all nine native control apply/reset checks.
Five interior removal probes still find no walkable area. A compiler regression
moves a deck away from a ground hole: the old gap becomes impassable while the
new floor remains connected, without adding ground sockets for either location.
151 affected tests, application typecheck and focused lint pass. Native moved
placements, rendered review and publication remain pending.

`check-bridge-asset-ground.mjs` converts the twenty rotated/elevated bridge
fixtures into asset-owned ground and removes editor terrain before compilation.
Stage `bridge-asset-ground-RnigNG` passes all forty native actor crossings and
3,120 sight/projectile queries. Forty one-unit height mismatches reject; ten
separated-bank removal checks retain disconnected banks. This tests arbitrary
bridge orientation on continuous ground and fitted banks, not native rendered
playthroughs of every possible placement.

Publication `bridge-publish-8TtzAK` installs the bridge and corrected background
definitions and refreshes Croisement03's descriptor pins. The bridge remains a
draft with explicit rendered actor occlusion/shadow warnings. Running
`check-saved-bridge-contact.mjs --published` produces
`saved-bridge-contact-dKPC8h`; both native actor directions pass using published
definitions without staged overrides.

Fresh inventory `gameplay-coverage-spEhsn` finds 1,279 indexed assets, 89 missing
definitions and 61 missing placed definitions: 57 Croisement02 assets, one
Croisement01 asset and Croisement03's two ferns and stream fallen log. The bridge
is no longer missing. The previous four-missing-placement count is historical;
the current library has changed. Definition presence is not proof of gameplay
completeness.

The foliage authoring script now accepts explicit `--asset=<id>` selections and
requires one-sided MASK materials with physical foliage opacity semantics.
Opaque wooden geometry and mixed-sided foliage do not silently enter that path.
The browser mask review now reads candidate IDs from the stage edits instead of
hardcoding two ferns.

Stage `fern-gameplay-wS3DZB` contains Croisement02 ground plants 111–123. Each
uses its own mesh alpha coverage, local ground contact and authored closed
canopy boundary. All 130 exports (thirteen assets, five rotations, two heights)
pass native construction. The authoring checks require nonempty typed masks,
coverage-spanning boundary polylines, rejection of a one-unit ground mismatch
and no generated movement obstacles. These definitions remain unpublished:
rendered character/projectile occlusion is unverified, and the closed canopy
boundary is an explicit approximation. The library missing-definition count has
not been reduced by these staged candidates.

Mixed-sided shrub authoring now opts into `preserveMaterialSidedness` during
mesh extraction. Double-sided materials receive opposite-winding copies of the
alpha-clipped triangles; one-sided materials retain one winding. The default
recovery path remains unchanged. Thirteen mesh/rasterization tests pass,
including mixed opaque primitives and partial-alpha reverse-face coverage.
Focused lint/format checks pass; pipeline typechecking still fails in unrelated
`shared/src/state-delivery.test.ts` fixtures.

Shrub 55 stage `fern-gameplay-pW8Bcl` passes ten native construction checks and
ten wrong-height rejections. Its browser point-mask review completes sixteen
cases at four rotations but fails the edge-consistency criterion at 0° and 90°.
Rendered leaf pixels without CPU mask coverage number 6/5/2/4 at 0/90/180/270°.
Clipped GPU versus CPU coverage differs at 2/12/9/27 pixels; clipped GPU versus
textured mesh differs at 8/15/5/26. This separates remaining extraction/rendering
and rasterization differences; it does not certify native compositing. The
candidate remains unpublished. The review sheet and reported case count now
scale to the selected asset list rather than assuming two ferns.

Ground plants 111–123 and shrub 55 are now published as best-effort drafts via
`ground-plants-publish-wS3DZB` and `shrub55-publish-pW8Bcl`. Reviewed model hashes
were checked against the current library before publication; descriptor pins
were refreshed without replacing scene content. The local canopy approximation
and rendered character/projectile review warning remain in each definition.
Pixel-level mismatches are retained as open evidence rather than treated as a
reason to omit otherwise usable draft foliage from exports.

Current Croisement02 omits its optional size. `check-saved-map-exports.mjs` now
derives static-model bounds from pinned placed vertices using the editor's
`contentBakeBounds` function and border convention. Generated terrain/splines
without explicit bounds still require browser verification. A focused fixture
checks placement, explicit frames, missing-size behavior and changed-model pins.
Export `saved-map-exports-FBLYbR` uses bounds [-231, -282, 2177, 1929], includes all
fourteen new masks and has no omission warning for the published plants/shrub.
Native construction passes with five areas, 154 sight obstacles, five doors,
four jump pairs, 8,575 grid blocks and 57 elevation boundaries. All nine native
control apply/reset checks pass. This remains geometry verification, not a
complete native rendered ZIP playthrough.

Fresh inventory `gameplay-coverage-io9QXo` finds 75 missing definitions among
1,279 assets and 47 missing placed definitions: 43 in Croisement02, one in
Croisement01 and three in Croisement03.

Batch `shrub-batch-erzjxjez` was intentionally stopped with `complete: false`:
sixteen assets passed ten native construction cases each; two authoring attempts
failed. Shrub 73 has UV values outside the supported sampler domain; shrub 74
fails the nearest-sampling/no-texture-transform precondition. Neither failure
was converted to empty gameplay.

More seriously, the first fourteen successful explicit-triangle definitions
total 468,916,952 bytes. Shrub 69 alone contains 719,673 triangles and occupies
127,141,905 bytes. These candidates were not published. Mesh UVs and compact
alpha coverage need to replace per-texel geometric expansion for a practical
library; successful native construction does not make this storage cost viable.
The already-published foliage drafts also need migration to that representation.

`configureAssetGameplay` now accepts a reviewed `modelSha256` for full gameplay
edits. It checks model bytes before staging and again before installation;
foliage authoring emits that hash. Five publication tests pass, including changed
model bytes with an unchanged descriptor. Focused lint and formatting pass;
pipeline typechecking remains blocked by unrelated state-delivery test types.

### Compact foliage alpha coverage candidate

`stage-fern-gameplay.mjs --compact --asset=croisement02-shrub-69` produced
`fern-gameplay-hQfhNB`. Asset definitions retain mesh triangles, UV coordinates,
material sidedness, cutoff, vertex alpha and texture alpha. The compiler samples
these after placement; exported native masks retain their existing bitmap format.
Spline splitting carries UVs and alpha through the same interpolation as geometry.

The staged mask occupies 18,106,822 bytes with 24,334 stored triangles, compared
with the previous 719,673 clipped triangles and roughly 127 MB definition.
All ten cases (0/37/90/180/270 degrees at elevations 0/40) have exactly the same
covered pixels as `fern-gameplay-SCMH3E`: no added or removed pixels. Each passes
native geometry construction and rejects a receiving ground one unit lower.
This measures representation fidelity, not rendered character/projectile parity.

The candidate is not published. Authoring still computes the clipped mesh
temporarily for its established canopy footprint; this does not yet solve the
unsupported sampler cases or authoring memory cost. Published foliage migration,
browser review support for compact coverage and further storage reduction remain.

### Compact coverage visual review and first draft publication

The browser reviewer now independently draws compact mesh UVs, alpha textures,
cutoffs and material sidedness on the GPU. It preserves diagnostic images on
failure and sizes its viewport from every tested mask's complete bounds; the
previous fixed viewport cropped large shrubs. Two helper regressions check
texture sharing, vertex sampling, sidedness, disposal and legacy opaque coverage.

Shrub 69's complete-bounds review ran 16 point-mask compositions across four
rotations. GPU compact coverage and the source textured model have zero differing
pixels at every rotation. CPU bitmap differences are 0/89/52/81 pixels at
0/90/180/270 degrees. At 90 and 270 degrees some discrepancies are isolated
rather than immediately adjacent to matching coverage, so the diagnostic fails.
Snapping vertices to 1/16 pixel reduces those differences to 0/36/28/36 but does
not eliminate them; no production rasterizer quantization change was made.
Images are in `fern-gameplay-hQfhNB/browser-review-full/after.png`.

`shrub69-compact-publish-hQfhNB` installs the reviewed asset definition and updates
the saved Croisement02 descriptor pin. It carries an explicit draft warning for
rotated CPU rasterization differences and the approximate canopy boundary.
Publication does not establish rendered gameplay parity. Other published foliage
still needs migration, and remaining missing asset definitions still need authoring.

The installed saved-scene export `saved-map-exports-31wLjX` contains 15 masks
and emits both shrub 69 draft warnings. Native construction passes with 5 areas,
154 sight obstacles, 5 doors, 4 jump pairs, 8,575 grid blocks and 57 elevation
boundaries; all nine existing transitions apply and reset successfully.

### Streaming wrapped foliage footprints

Compact authoring now streams alpha-clipped triangles into a hull accumulator
in batches of 3,000 points. It no longer retains the full expanded asset mesh.
Repeat and mirrored repeat UVs are split into their intersecting tiles, preserving
interpolated positions and vertex alpha. Excessive repetition above 4,096 tiles
per triangle reports an error. Clamped edges retain their exterior texels.
The explicit nearest base-level authoring mode matches map baking even when a
source material requests filtering; texture transforms remain unsupported.

`fern-gameplay-BZ5rMA` checks shrub 68. Its boundary and ten rotated/elevated
bitmap exports match `fern-gameplay-3HULzB` exactly. It stores 3,928 triangles
instead of 59,474, in a 3,746,085-byte mask definition. `fern-gameplay-GO93gA`
successfully authors previously blocked shrubs 73/74: respectively 20,840/7,405
stored triangles and 15,261,157/5,011,969 bytes. All thirty native construction
cases pass and all thirty incorrect receiving heights are rejected.

`compact-shrubs-publish-BZ5rMA-GO93gA` installs these three definitions and updates
the saved Croisement02 pins. All retain draft warnings for approximate canopy
boundaries and unverified rendered occlusion/projectile contact. Twenty-four
focused tests pass, including streamed geometry equivalence and negative/repeated
UV footprints. This does not resolve the CPU/GPU bitmap edge differences or
complete the remaining foliage migration.

The installed full-scene export `saved-map-exports-w0yeJQ` contains 18 masks.
Native construction and all nine control apply/reset checks pass. Pipeline
typechecking still reports the existing state-delivery test errors; focused lint
and formatting pass.

### Deduplicated foliage alpha planes

Compact staging initially failed for shrub 57 (`fern-gameplay-A05eJC`) because
separate texture objects repeated the same 1,152×2,112 alpha plane enough times
to exceed the 64-million-texel decoded coverage limit. Identical alpha contents
and dimensions now share one serialized plane, while triangle UVs, cutoff,
wrapping and sidedness remain independent. Identical encoded images also share
their decoded alpha during extraction. Eight focused recovery tests pass,
including distinct dimensions with identical bytes and differing material rules.

`fern-gameplay-vyNGFf` authors shrubs 57–60 with 4/4/4/3 unique alpha planes and
6,064,531 / 4,641,548 / 4,442,239 / 6,205,455-byte mask definitions. All forty
rotated/elevated exports pass native construction and reject an incorrect
receiving height. `compact-shrubs-publish-vyNGFf` publishes these as warned drafts
and updates saved Croisement02 pins. Rendered occlusion/projectile contact and
CPU/GPU rasterization differences remain unresolved.

The subsequent inventory `gameplay-coverage-ieNAcG` finds 67 of 1,279 definitions
missing, including 39 placed assets. Definition presence is not proof of complete
gameplay behavior; 26 missing assets have potential ownership candidates that
still require local geometry/control review.

Installed export `saved-map-exports-ZkIZxm` passes native construction and all nine
control apply/reset checks. It has 21 masks: shrub 60 is omitted with an explicit
warning because its placed origin has no authored receiving surface at its ground
height. Its ten isolated terrain cases pass, but the saved scene requires further
receiver/placement authoring. This omission is not hidden by the successful native
construction result.

### Asset-local mask receiving points

Shrub 60's origin is unsupported, but multiple canopy-footprint vertices overlap
the same surrounding ground layer. Masks now accept explicit `receiverPoints`
in asset-local coordinates, mutually exclusive with segment/polyline probes.
Placement transforms each point; only contacts at the authored surface height
contribute, and all contributing receivers must resolve to one layer. No nearby
layer or source-level fallback is used. Foliage authoring emits the ground origin
and alpha-covered footprint vertices as receiving points.

Spline deformation transforms and trims these points with each repeat. A surviving
point retains a mask whose representative origin was cropped; a repeat with no
surviving contacts is omitted with a warning. Tests cover unsupported origins,
wrong heights, competing layers, rotated duplicates and cropped spline contacts.

`foliage-contacts-EQHthn` stages shrub 60's contact definition;
`saved-map-exports-DRzWhE` restores its mask, producing 22 masks without a shrub 60
receiver omission. Native construction and control apply/reset checks pass.
`foliage-contacts-publish-EQHthn` installs the definition and scene pin. This fixes
receiving-layer selection, not the remaining rendered bitmap discrepancies.

### Published compact ferns and shrubs 62–67

`fern-gameplay-BL4YZw` derives Croisement03 ferns 35/76 from their pinned assets,
using compact alpha and explicit ground-contact points. Twenty rotated/elevated
exports pass native construction and reject one-unit receiving-height mismatches.
The masks occupy 5,514,315 and 3,611,041 bytes. Full-scene stage
`saved-map-exports-wXMFi3` retains three masks, including both ferns, without
receiver omissions. Native construction and all nine transition checks pass.
`compact-ferns-publish-BL4YZw` installs the definitions and saved-scene pins.

`fern-gameplay-uPfUI9` derives Croisement02 shrubs 62–67. All sixty placement
exports construct natively and reject incorrect receiving heights. All sixty
bitmaps match their previous clipped definitions exactly, with source model
hashes verified equal before comparison. Full-scene stage
`saved-map-exports-JlmjWo` contains 28 masks and no receiver omissions for this
batch; native construction and nine transition checks pass.
`compact-shrubs-publish-uPfUI9` publishes these definitions and scene pins.

All eight assets remain drafts: closed canopy boundaries are approximations and
rendered character/projectile contact is unverified. The subsequent inventory
`gameplay-coverage-3RzSpw` finds 59 missing definitions among 1,279 assets, with
31 placed in saved scenes. Only the stream fallen log remains without gameplay
in Croisement03; definition presence does not establish full map parity.

### Remaining Croisement02 foliage draft publication

Four compact stages cover 25 previously missing foliage definitions:

| Stage | Assets | Placement/native cases |
|---|---|---:|
| `fern-gameplay-IiW8JD` | Shrubs 70–72 and 75–77 | 60 |
| `fern-gameplay-vn3AlI` | Shrubs 78–83 | 60 |
| `fern-gameplay-fcZjIO` | Shrubs 84–89 | 60 |
| `fern-gameplay-gIA8lV` | Shrubs 90–93, canopy fringe 22, northwest boundary shrub 54, west shrub bank | 70 |

All 250 exports pass native bitmap decoding, mask layer registration, obstacle
link checks and geometry construction; all reject receiving ground displaced
down by one unit. Shrubs 70–72 and 75 additionally match the earlier clipped
bitmaps exactly across forty placements, with equal source model hashes.
The definitions retain 226,389 source triangles instead of 4,574,110 expanded
triangles; their serialized mask metadata totals 184,369,042 bytes. This remains
a material storage cost, despite the reduction in geometry expansion.

Combined stage `foliage-combined-qUMEcd` produces saved-scene export
`saved-map-exports-qR9I0v`: 52 masks, successful native construction and nine
successful control apply/reset checks. Shrub 77 still has no matching receiving
surface in its saved placement and is omitted with an explicit warning. Other
warnings include four missing wooden/fence definitions and collapsed movement
holes; these successes do not establish full scene parity.

The four `compact-foliage-publish-<stage suffix>` reports install all 25 definitions
and update saved scene pins. Each retains warnings for approximate canopy
boundaries and unverified rendered character/projectile contact. No visual
parity claim is made for this batch.

Post-publication inventory `gameplay-coverage-cykIvk` finds 34 missing definitions
among 1,279 assets, six placed: Croisement02 fences 94/95 and supplemental wood
09/44, Croisement01 tree 03, and the Croisement03 stream fallen log. Twenty-six
missing library assets have possible component owners but still need local
geometry/control review. Shrub 77's receiving failure is a separate known issue
and is not included in the missing-definition count.
