# Runtime changes introduced or extended for map compilation

Audit date: 2026-10-08. Code inspected through `beec74bc8`.

**Static navigation preparation has incorrectly entered movement ticks.**
The Silver Arrow frame-5 freeze reported against `9920d51a5` is a release-blocking
regression. Passing small exported-map traversal fixtures did not establish
acceptable runtime cost or compatibility with populated stock missions.

This inventories the map-compilation work beginning with `1a5ba4ee3`, including
mission authoring and the engine extensions subsequently introduced for traversal.
Concurrent multiplayer, combat, campaign, diagnostics and unrelated renderer work
are not attributed to map compilation merely because they touched the same files.
Commit references below identify introduction and subsequent changes, rather than
implying that every line in each commit changes production behavior.

The tables below are a **historical inventory at the audit baseline**, not a list
of code still present. Following the audit, the graph-less visibility fallback
and its dispatch branch were deleted outright. The separate physical navigation
solver has now also been deleted: walking and stair route reconstruction,
landing/floor binding, physical collision neighbours, movement dispatch,
per-step execution, order fields and runtime door endpoints. Ordinary prepared
graph routing and movement remain. No compatibility replacement was added.

The proposed caching/relocation remedies in the historical tables are superseded
by that deletion. A small compiled-data extension may be justified by a concrete
export requirement; a second runtime solver is not the intended architecture.
Static descriptor validation remains in the level-data crate.

Export now generates the native graph in
`level-editor/shared/src/compile-navigation-graph.ts`. It constructs actor-clearance
links and state constraints from placed motion polygons during export. The game
only loads the prepared stream and uses its existing pathfinder. The graph builder
uses the same integer collision contours as the native grid and rejects boundary
contact. It currently supports the stock 6×3 half-diagonal and the existing
65,535-link stream limit. Best-effort overflow is an explicit warning and omitted
graph, not a compatibility solver.

Current validation: the game builds; 213 selected editor compiler/export tests
pass; native map integration reports **69 passed, 5 ignored**. The engine suite
reports **4,335 passed, 0 failed, 33 ignored**. Stair receivers,
overlapping/copied traversal and changing stair barriers now pass with prepared
native endpoints. Earlier physical traversal results do not certify this
architecture. Authorized direct-route probes now enable the native direct check;
forcing graph-only search for an already clear segment is not the request contract.

Passage endpoint preparation now runs in `compile-lift-approaches.ts` before
descriptor/ZIP creation. It respects permanent blockers, retains shared approaches
for switchable barriers, and preserves climbing animation radii. Unavailable
actor-sized approaches produce explicit warnings. Physical construction metadata
is removed from exported lifts. The loader's approach search and its module were
deleted; the native tests consume exported points without startup repair.

The saved Derby scene compiles a **613,728-byte graph**, with no graph omission
warning, in roughly 4.6 seconds of export-time graph preparation. Its descriptor
constructs 60 areas, 848 sight obstacles, 70 doors and 2 jump pairs. Native sampling
passes **144 ordinary routes**, spending 0.01 seconds in pathfinding in the
unoptimized harness. Evidence: `work/map-compile/saved-map-exports-GEMe4m` under
the editor directory. Its 268 other authoring warnings remain; this is not full
gameplay parity.

Wychford exposed the native graph capacity limit: export spent approximately
337 seconds preparing the graph before exceeding 65,535 links, and 571 seconds
overall. Best-effort output explicitly warns that its graph was omitted and
indirect routes are unavailable. Evidence:
`work/map-compile/saved-map-exports-EPZ3ds`. This is an unresolved compiler/format
gap, not evidence of working Wychford navigation. Other large scenes remain open.

The pipeline-wide TypeScript check currently fails in `state-delivery.test.ts`
(presentation-frame fields and assertion/narrowing types), outside these changes.
Focused lint of the modified compiler/test files passes.

The optimized `parity` profile passes
`game_session::multiplayer::story_regression::stock_three_player_navigation_tick_cost`
using full GOG data, active mission scripts, three PCs and 180 frames per mission.
All 540 frames complete. Times measure `advance_frame_without_hash`; startup,
rendering and logging are outside the timed region, and frame 0 is excluded from
the distribution.

| Mission | Prepared nodes | Frame 5 | Median tick | p95 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| H07_Not_MK | 891 | 3.107 ms | 2.750 ms | 3.426 ms | 4.772 ms |
| H01_Lin_VL | 633 | 0.847 ms | 0.927 ms | 1.363 ms | 5.989 ms |
| S01_Not_VL | 891 | 1.770 ms | 2.834 ms | 3.367 ms | 4.707 ms |

The pre-removal reproduction stalled at Silver Arrow frame 5 until a 60-second
external timeout. That reproduction used the unoptimized test profile, so these
measurements establish completion, not a directly comparable speedup ratio.
They do not establish a universal 1 ms tick budget or measure exported-map routing.

## Required division of work

* **Export:** construct placed map geometry, connections, navigation nodes/links,
  actor-clearance geometry, state-dependent link constraints, receiver ownership,
  collision indices, elevation seams and artwork variants from assets and scene.
* **Mission start:** validate/decode those products, bind runtime indices, select
  character/profile configurations, initialize spatial indices and mission actors.
  Missing compiled navigation must not select a compatibility routing algorithm.
* **Path request:** connect the current source/goal to prepared navigation and
  search it with deterministic, bounded scheduling. Retain the resulting route.
* **Movement tick:** advance the retained route, evaluate the current plane,
  check local moving obstacles and active state, and perform gameplay callbacks.
* **State change:** activate/deactivate prepared geometry and links; invalidate
  affected routes. A door switching state does not make its shape newly authored.

Exported profile-dependent data needs a version/configuration identity. Profiles
not known to the exporter can require mission-start preparation. This does not
justify deferred first-use preparation in a movement tick. Dynamic avoidance
must use local queries; a real replan belongs in the path-request system.

## Navigation and collision inventory

Paths in this table are relative to `crates/robin_engine/src/` unless prefixed.

| Added or extended behavior | Execution at audit baseline | Original audit recommendation (superseded for the deleted solver) | Implementation / commits |
| --- | --- | --- | --- |
| Ordinary physical-floor routing | Every moving actor step with `physical_walking`; rebuilds a collision snapshot, support, clearance region and route to the same goal. Ordinary receiver bindings are built for stock missions too. This is the reported freeze path. | Remove static reconstruction and whole-route search from ticks. Export/load prepared routing, retain route progress, invalidate on relevant changes. | `engine/movement/physical_walking.rs::commit_physical_floor_step`; `engine/movement_step.rs`; `9920d51a5` |
| Ordinary floor discovery | Command extraction/dispatch searches all bound floors. `contains_world_position` constructs and validates boundary/hole polygons during queries. | Prepare polygon objects and a receiver/layer/sector-to-floor index at export/load. Keep position/height checks local. | `engine/movement/physical_walking.rs::current_physical_walking_floor`; `stair_navigation/walking_binding.rs`; `f006c2dce`, `9920d51a5` |
| Ordinary floor collision snapshots | Each query clones boundary, holes and active obstacle rings; validates them again in `PhysicalWalkingSurface::geometry`. | Immutable prepared geometry plus state selection; validate once at load, not repeatedly per actor. | `stair_navigation/walking_binding.rs::snapshot`; `stair_navigation/walking_surface.rs`; `e55d957b4`, `f006c2dce` |
| Neighbouring walking-floor support | Neighbour identities are prepared at load, but every query clones their snapshots and subtracts active obstacle polygons again. | Export/load static support and blocker dependencies; state masks or affected-state updates must replace repeated Boolean subtraction. | `stair_navigation/walking_binding/neighbours.rs::neighbour_support`; `2a20eede6` |
| Stair support supplied to ordinary walking | Every ordinary walking snapshot scans all physical stairs; matching flights rebuild polygons and subtract active obstacles. | Prebind floor-to-flight adjacency and prepared support; select by live state. | `engine/movement/physical_walking.rs::physical_walking_geometry`; `stair_navigation/landing_binding.rs::walking_support`; `2a20eede6` |
| Physical stair/wall/ladder routing | `commit_physical_stair_step` calls `route_with_obstacles` before each step. It clones static shapes, rebuilds landing support and recomputes routing. Explicit physical navigation enables this branch. | Same prepared-data and retained-route correction as ordinary walking. Do not fix just the stock-mission caller. | `engine/movement/physical_stair.rs`; `stair_navigation.rs::BoundPhysicalStair::route_with_obstacles`; `7ad10c85e`, `91230136d`, `7f2b7946b`, `ae85a6144` |
| Configuration-space support/visibility solver | Per route: clips support, unions landings, buffers roundoff, expands solids, erodes floor support, selects a region, buffers it, enumerates boundary vertices, then repeatedly performs polygon/line relations during visibility search. | Export/load clearance regions and indexed visibility/connectivity. Query work should attach endpoints and search prepared links. Endpoint rounding must not force rebuilding a whole buffered region. | `stair_navigation/landing_support.rs::route_on_surface`, `clearance_centers`; `5db666c93`, `d8c2f4c56`, `4cee092eb`, `e55d957b4` |
| Separate stair solver without landing support | `StairRouteGeometry::route` constructs a new `FastFindGrid`, registers all boundaries/obstacles, creates a `PathGraph` and initializes a `PathFinder` for each query. It is selected for a non-climbing stair without bound landings. | Remove per-query static grid/graph construction. Use prepared local navigation or the compiled map graph. | `stair_navigation.rs::StairRouteGeometry::route`; `219244367` |
| Graph-less projected routing fallback | At path-query time, generated offset corner candidates, authorized them and discovered visibility while searching. Introduced for editable JSON maps that omitted the precomputed graph; assumed only tens of corner candidates. Predated map-export work, but was expanded for concave boundaries, narrow walkways and A* ordering. | Deleted after this audit; no compatibility path. Export must generate the missing graph. | Former `pathfinder.rs::find_path_visibility_fallback`; introduced in `2e0379d3a` (2026-08-19), recovered from archived Git history; expanded in `3575ba3ed`, `137c1108c`, `1b3b83275` |
| Walking source recovery | On movement-command extraction, unions support, erodes full/recovery footprints, subtracts solids and tests candidate recovery sweeps. Can also be reached through movement extraction after state changes. | Prepare allowable support/clearance; retain bounded current-position recovery as a query. Do not repeatedly construct static recovery regions. | `engine/movement/elevation.rs::extract_move_instruction_owner`; `stair_navigation/walking_surface.rs::recover_source`; `e55d957b4`, `9920d51a5`, `b3691f9ff` |
| Stair source authorization | Command/approach checks call the full route solver with identical source and goal merely to establish footprint support. | Prepared point/footprint authorization query. No graph building for a stationary support check. | `engine/movement/elevation.rs`; `engine/movement.rs`; `18cce7dcf`, `38f3a1a05` |
| Physical dynamic actor avoidance | Every physical step gathers actor/object neighbours, using an entity scan followed by filters. Walking converts accepted neighbours to 16-sided polygons and feeds them into whole-floor routing. | Keep live avoidance, but use local spatial candidates and a local swept-step/corridor test. Do not combine each moving actor with a fresh global Boolean/visibility solve. | `engine/anti_collision.rs::gather_physical_stair_neighbours`, `gather_physical_walking_neighbours`; physical movement modules; `7f2b7946b`, `9920d51a5` |
| Climbing footprint transformation | Recomputes plane-adjusted offsets; compound flights compute their convex envelope, including during movement/obstacle checks. | Prepare offsets/envelopes per supported actor footprint and floor configuration. Only translation to current actor position is live work. | `stair_navigation/clearance.rs`; `3b6283661` |
| Piecewise physical floors | Validates connected floor patches at construction. Queries recreate patch polygons for point selection; stepping/distance queries intersect the current segment with patch edges. | Keep evaluating current position and advancing in physical distance; prepare patch polygons, adjacency and spatial edge lookup. Route segments can retain patch transitions. | `robin_level_data/src/stair_navigation_floor.rs`; `8e7c4fe61`, `b17710a08` |
| Physical stepping and animation increments | Advances along the chosen route in physical XY/Z, sets movement forecasts/direction and preserves distance through animation transitions, including edge-on projections. | These are legitimate per-tick operations **once a route is prepared**. Keep narrowly scoped to maps/paths requiring physical geometry. | `robin_level_data/src/stair_navigation.rs`; `position_interface.rs`; `sprite.rs`; `engine/transitions.rs`; `b2d8b47cc`, `f301f62f3`, `ef01ed0b9`, `677ec64d7` |
| Physical order dispatch, seek and receiver handoff | Adds physical destination/floor identities, same-floor shortcuts, precise door endpoints and receiver retention through entry/exit/animation callbacks. Later fixes restrict destination layer/receiver. | Keep necessary runtime selection/ownership, backed by compiled floor/door bindings. Reassess broad stock-map dispatch introduced by ordinary walking. | `engine/movement.rs`, `engine/door_pass.rs`, `engine/movement/elevation.rs`, `order.rs`; `134921667`, `2a9978717`, `9ff58f63c`, `efc1efc80`, `3876470a7`, `721022919`, `d24f69096`, `c21128e17` |
| Mouse movement authorization | Rejects loaded movement obstacles and resolves physical stair destinations; handles ambiguous/edge-on projections without choosing an arbitrary height. | Runtime input query is appropriate; use prepared geometry and indices. | `engine/input.rs`, `robin_rs/src/host_mouse.rs`; `04216bf4a`, `38f3a1a05` |
| Fractional elevation seams and crossings | Preserves fractional seam endpoints, handles partial edge cells and receiver crossings at junctions/ground; queries occur during movement. | Compile the seam topology and its index; keep local crossing/height/material updates at runtime. | `engine/level_loading.rs`, `engine/movement/elevation.rs`; `2988b81dd`, `04c2aeda7`, `6a6ca380f`, `7708dbd47`, `9db07570d` |
| Fast-find corridor optimizations | Uses first-blocker exit, skips empty cells and narrows candidate cells to intersected row ranges. Used by route/reachability queries on both old and new maps. | Appropriate runtime queries over static indices. These optimizations do not justify rebuilding a visibility graph. | `fast_find_grid.rs`, `pathfinder.rs`; `1b3b83275`, `0c29e3ab3` |

The ordinary walking path currently also has an explicit unfinished soft-repulsion
TODO. Its behavioral parity was not complete before the performance regression.
Do not silently accept a cached path through newly active obstacles: state changes
must invalidate or disable affected links, and the next local step must check the
current state. That requirement does not require rebuilding static shapes.

## Load-time construction inventory

These are runtime additions too, although they do not run on every movement tick.

| Added or extended behavior | Current execution | Required disposition | Implementation / commits |
| --- | --- | --- | --- |
| Compiled descriptor and native record conversion | Reads `asset_geometry`, validates topology/references and constructs motion, sight, material, door/lift/building, jump, lighting, sound, mask, scenery and control records. | Keep decode/validation and native binding. Export should supply finished static products, rather than asking the loader to repair authored geometry. | `robin_level_data/src/level_data.rs`; `16146c067`, `4a206e09f`, `1bee3ee80`, `cc95b8016`, `35683e261`, `21f0c532f`, `a288b0b97` |
| Receiving-boundary derivation | When `elevation_lines` is empty, computes boundary intersections, ownership and lift connections on JSON load; later added spatial indexing. | Move to export-time generation; distinguish an intentionally empty result from missing compiled data. | `robin_level_data/src/compiled_elevation.rs`; `2988b81dd`, `62ccb364d` |
| Lift/ladder/wall approach repair | JSON loading calls `compiled_approaches::derive`, searches nearby authorized approaches and can change door points. Explicit physical-navigation lifts skip this repair. | Move authoring correction to export and report it there. Loader should validate finished endpoints, not silently redesign the approach. | `robin_level_data/src/compiled_approaches.rs`; `cf5847fb6`, `be4d0c2c8`, `4151993ae`, `8d352340e`, `902a8432b` |
| Ordinary receiving-floor binding | Groups receivers by plane/area, unions footprints, intersects motion coverage, unprojects obstacles and computes compatible neighbours. Runs during motion initialization, including stock maps. | Export prepared physical floors/adjacency where required; load-time adaptation only for formats that actually need it. Preserve stock precomputed navigation behavior. | `engine/level_loading.rs::bind_physical_walking`; `stair_navigation/walking_binding.rs`, `walking_binding/neighbours.rs`; `f006c2dce`, `2a20eede6` |
| Physical flight/landing binding | Validates definitions, binds obstacle states and door receivers, clips landing support/holes/collision and handles shared-edge rounding. | Most is static export work. Keep load-time ID binding and validation; reuse the prepared result in queries. | `engine/level_loading.rs::bind_physical_stairs`; `stair_navigation/landing_binding.rs`; `robin_level_data/src/physical_stair.rs`; `efdb2a502`, `91230136d`, `ca6e13cbf`, `ae85a6144` |
| Precise geometry and plane validation | New optional precise motion contours, physical floor patches, world door endpoints and independent receiver anchors. Includes ring validation and rounding-aware checks. | Appropriate format support and load validation. The current query helpers must stop repeating static polygon validation. | `robin_level_data/src/level_data.rs`, `physical_stair.rs`; `stair_navigation/ring_validation.rs`; `e4794cdb8`, `edf32ad8c`, `c00098867`, `a53ea3ceb`, `c7d686a6a`, `e3044da75` |
| Lift endpoint and passage bindings | Uses placed landing heights/explicit identities rather than screen-Y ordering for compiled lifts; stores precise door points and high/low door indices. | Export identities/geometry, bind IDs at load. Runtime reads those fields for entry/exit/fall behavior. | `engine/level_loading.rs`, `gate.rs`, `fast_find_grid.rs`; `4e66f4f65`, `795413f04`, `3876470a7` |
| Material and receiving-plane initialization | Distinguishes an explicitly empty material selection from an absent list; consumes explicit plane anchors and initial sight activation flags. | Keep loader interpretation of exported data. | `engine/level_loading/environment.rs`; `1bee3ee80`, `a6bc64ca5`, `b14ad3dc2` |
| Typed masks and links | Validates encoded mask bounds, bitmap data, types, polylines and obstacle references; resolves initial/applied mask links. | Appropriate loader work. Bitmap/polylines and ownership are exporter output. | `robin_level_data/src/compiled_masks.rs`, `level_data.rs`; `44bf34bc0`, `a32d3bd57` |
| Empty unscripted interiors/map interactions | Builds doors/lifts/buildings without requiring mission population or script state; door/gameplay callers use the map's live interactables. | Necessary separation of map from mission, retain it. | `engine/ai/initialization.rs`, `engine/movement/{routing,formation,door_traversal}.rs`, `engine/refresh_seek.rs`, `engine/sequence_validity.rs`, `engine/tick.rs`, `engine/jump.rs`, `engine/corpse_intersection.rs`, input/render accessors; `4a206e09f`, `cf5847fb6`, `4151993ae` |
| PC spawn slots and NPC placement | Loads canonical `spawn_points`, profile overrides, layers/sectors/receiver references and soldiers; accepts older spawn input and uses authored roster at startup. | Mission-start behavior, appropriate. Keep authored mission content separate from reusable map geometry. | `robin_level_data/src/level_data.rs`; `engine/level_loading/pcs.rs`; `7334c129c`, `4ea55dba1`, `bce676a20` |
| Named mission import/loading and archive precedence | Loads named mission records, keeps names/handles for scripts, and resolves archive level overrides before base data. | Appropriate import/load behavior; no map geometry reconstruction per tick. | `robin_level_data/src/level_data.rs`; `engine/level_loading.rs`, `engine/level_loading/entities.rs`, `natives/handle_codec.rs`; `robin_rs/src/game_session/setup.rs`; `f0ef9e75c` |
| Scenery record/resource loading | Converts compiled animation groups and sprite names, loads packaged banks into the startup prototype cache, including namespaces for collisions. | Export placement/resource tables; decode and preload at startup; existing animation playback remains live. | `robin_level_data/src/level_data.rs`; `robin_assets/src/custom_sprites.rs`; `robin_rs/src/game_session/setup/custom_sprites.rs`; `e13eb1acc`, `1e532fc9e`, `67d0a2e87`, `3952dea0c`, `e45b2b13b`, `f080783dd` |

## State changes, presentation and compatibility inventory

| Added or extended behavior | Current execution | Required disposition | Implementation / commits |
| --- | --- | --- | --- |
| Multi-area movement switches | Applies additional motion changes across areas, converts sector IDs, validates duplicate bindings and toggles existing pathfinder/grid state. Invalidates affected actor movement. | Keep state toggles/invalidation. Resolve static binding identities and duplicates at load. Export the dependency table. | `engine/patch_effects.rs`, `patch.rs`, `robin_level_data/src/level_data.rs`; `2dda94cb2`, `ba146b03c`, `22e33d77a` |
| Sight/receiver state changes | Consumes explicit initial/applied sight sets, including receiving volumes, while keeping floor ownership separate from visibility. | Keep state selection over prepared sight geometry. Avoid recomputing receiving topology on state changes. | `engine/patch_effects.rs`, `engine/level_loading.rs`; `a288b0b97`, `871b1bc96`, `d092c36e0`, `62c2adb53` |
| Door rights, passage masks and visual-only controls | Applies/reset permissions and baked masks through linked doors; allows appearance-only switches without artificial collision side effects. | Appropriate event-time application of compiled bindings. | `engine/patch_effects.rs`, `robin_level_data/src/level_data.rs`; `552302220`, `44bf34bc0`, `6ee23c2b6` |
| Closing barriers against physical actors | Includes actors supported by connected landings when invalidating routes and testing newly appeared barriers/crushing. Uses transformed physical footprint. | Keep current-actor intersection and gameplay effects. Prepare barrier geometry/footprints and dependency indices. | `engine/patch_effects.rs`, `stair_navigation/landing_binding.rs`, `clearance.rs`; `45f66cea2`, `3b6283661` |
| Baked appearance state resources | Loads and validates pre-baked region/state color and depth PNGs; initial renderer installation allocates backing buffers. | Appropriate startup decode. The expensive visual construction is already in export. | `robin_rs/src/level_loading_host/appearance.rs`, `level_loading_host.rs`; `b59162d54` |
| Appearance state synchronization | Each rendered frame polls region control bits. When changed, copies affected CPU regions then uploads full-map color and depth buffers. Does not rebake geometry or artwork. | Keep state selection; improve to dirty-region/GPU state selection if profiling warrants. Full-map upload on a small change is a separate identified inefficiency, not the reported navigation freeze. | `robin_rs/src/map_appearance.rs::sync`, `renderer.rs::sync_map_appearance`, `game_session/render.rs`; `b59162d54` |
| Export depth PNG decoding | Corrects loading of baked depth data; tests exercise native masking. | Keep decode at startup; no per-tick image construction. | `robin_rs/src/level_loading_host.rs`; `ff9f4d37a` |
| Serialization/replay/shipping contracts | Adds physical order/floor/door fields, navigation assets and associated schema compatibility changes. | Retain necessary state/version compatibility. Prepared static data should be owned as assets, not recopied as dynamic actor state. Audit format migration alongside removal/replacement of physical orders. | `order.rs`, `engine/level_assets.rs`, `engine/snapshot.rs`, `replay.rs`; `robin_assets/src/shipping_datadir*`; traversal/mission schema commits including `b17710a08`, `9920d51a5` |

New light/sound/material/jump definitions principally feed existing runtime
systems. Their loader and tests changing does not mean that a new simulation
algorithm was added. In particular, jump ledge/target generation, connection
matching and flight-clearance authoring checks are compiler work; the exported
jump tests do not establish a new per-tick jump-target generator.

## Test-only changes must not be counted as runtime algorithms

Many commits added native probes inside production source files under test
configuration. Examples include the sight/impact audit in `sight_obstacle.rs`,
scenery ordering checks in `engine/display_state.rs`, jump flight audits in
`engine/jump.rs`, and GPU checks in `renderer/gpu_contract_tests.rs`.
They execute in tests, not in the shipped movement loop. The fixture-only
rebind change `576f27875` is in this category too.

Likewise, the recently added fragment appearance ownership is exporter/library
metadata. It uses the earlier baked appearance consumer; it did not add another
runtime renderer. Spline height slicing, jump deduplication and the recent
actor-sized clearance probes did not change runtime movement behavior.

## Correction priorities and acceptance criteria

Historical rationale: `2e0379d3a` explicitly introduced the visibility fallback
alongside the first hackable JSON level descriptors because those descriptors
contained no precomputed graph. It enabled small unscripted sandbox maps. That
explains its introduction, not a requirement to retain it. The user explicitly
rejected compatibility support: remove the fallback entirely and make exports
supply the prepared graph. This fallback is separate from the later physical
routing solver implicated in the frame-5 freeze.

1. **Stop the regression in existing missions.** Preserve their prepared routing
   and avoid diverting them into a per-step physical visibility solver. Prove the
   Silver Arrow three-player frame-5 reproduction progresses on stock data with
   no networking dependency. Do not equate this compatibility correction with
   fixing exported maps.
2. **Produce prepared navigation for exports.** Nodes/links, clearance classes,
   floor support, receiver bindings, seams, bounds and state dependencies belong
   in a versioned compiled map product. Prefer using the existing graph machinery
   where it represents the required geometry. Any necessary physical extension
   needs a demonstrated geometry requirement and the same preparation lifecycle.
3. **Keep the physical movement solver removed.** Its support/solid Boolean
   operations, buffering, erosion, polygon validation, local grid/graph
   construction and visibility discovery must not return in movement or an
   unbounded first path request. Compile static navigation into the map.
4. **Retain efficient live behavior.** Local actor/object avoidance, current
   state-mask checks, bounded route requests, receiver updates and animation
   stepping remain live. Prepared links need obstacle/state dependencies so that
   changes, reset, save/load and replay restore cannot use stale clearance.
5. **Validate representative scale.** Stock populated missions plus large
   exported scenes; moved/rotated/elevated assets; flat/steep/compound floors;
   walking and climbing; narrow passages; opening/closing multiple doors; moving
   actors; restored state. Record startup cost, prepared-data size, route-query
   cost, worst movement-tick cost and static-preparation counters.

The structural acceptance condition is **zero static navigation preparation in
movement ticks**. Timing thresholds must then be measured at realistic scene and
actor counts. Existing small correctness fixtures remain useful, but are not a
substitute for either condition.
