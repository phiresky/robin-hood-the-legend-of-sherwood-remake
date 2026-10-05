# Post-port Features

- **Best-effort exports retain independent mask and traversal features.** If a
  placed mask boundary collapses in projection, export warns and omits that
  boundary rule while preserving usable view and obstacle-based rules. A stair
  region split by collision now uses the existing traversal-omission fallback,
  retaining collision and independent floors. Strict export still rejects both
  conditions; neither fallback certifies complete gameplay.

- **Physical clearances on shared navigation planes.** Asset receiver anchors and
  movement clearances may specify a local `navigationHeight` independently of the
  physical footprint's height. Projection happens after placement, so raised
  platforms keep their openings when rotated. Clearances remain limited to their
  own asset's collision and do not create floors. Spline deformation reports
  separate clearance heights as unsupported instead of silently changing them.

- **Asset placement ground height.** An optional asset-local
  `gameplay.placementGroundHeight` aligns new drops with terrain at a reviewed
  entrance/ground elevation, allowing foundations below that elevation. Assets
  without it retain lowest-geometry placement. Saved placements keep their
  existing transforms; compilation does not silently reposition them.

- **Wall export follows source deformation.** The editor derives spline source
  measurements from its loaded asset models before export, including source
  rotation, trimming and cross-section straightening. Physical volumes and
  walkable surfaces use those measurements while retaining asset-local gameplay
  definitions and original editor resource pins. Invalid source sections warn
  without discarding valid paths that use the same asset. Exact subdivision
  endpoints prevent floating-point clipping from dropping a terminal wall band.

- **State-dependent stair barriers.** Asset-local movement controls can bind to
  stair surfaces after placement, including slopes, rotations, elevation and
  copies. Controls retain separate generated state bindings. Mutually exclusive
  barriers no longer prevent constructing actor-sized stair approaches: permanent
  collision constrains the shared anchors, and traversal enforces active barriers.
  Unsupported changing
  ladder/wall barriers remain explicit errors in strict compilation; best-effort
  exports omit their controls with warnings and retain the initial state.
  Best-effort exports also retain excess controls' initial states when more than
  sixteen independent movement switches overlap one navigation area, reporting
  each omitted control instead of aborting the map export.

- **Mission preload for packaged scenery.** Compiled maps load their referenced
  custom Day animation banks from mod directories or ZIPs before creating effects.
  Scenery selection is independent of the character roster; unrelated animation
  banks are not decoded. Frames use the same validated runtime installation as
  custom characters.

- **Compiled scenery construction order.** Editor-authored animations now register
  their construction group alongside map controls, allowing maps containing effects
  to complete native loading. ZIP acceptance decodes packaged sprite banks and
  checks their placement and frame playback without a base datadir. Editor
  previews use the native loader's action/direction row ordering.

- **Live placed scenery previews.** Pinned animation assets now play in the map
  viewport, following asset moves, rotations, elevations and copies. Their
  billboards can be selected and dragged like model assets, contribute to view
  framing, and stay out of baked map artwork. Copies share decoded resources;
  unavailable banks produce a visible marker and warning. Frame delays and pixel
  offsets follow runtime placement rules.

- **Reusable scenery profile extraction.** An offline authoring command selects
  one complete sprite profile from a multi-profile PNG bank or a preview atlas,
  producing a self-contained PNG bank for pinned scenery assets. Frame timing,
  offsets, directions, profile centers and color-key semantics are preserved.
  Preview atlases remain previews; extraction cannot restore omitted frames.

- **Spatially indexed receiving boundaries.** Native loading of editor maps
  narrows edge intersections and receiving-plane queries with spatial bounds.
  Exact intersection tests, subpixel probes and receiver precedence are retained,
  so dense editable terrain does not require scanning every triangle for every
  boundary. Regression tests compare complete results against exhaustive scans.

- **Lift endpoints independent of map orientation.** Editor exports carry low/high
  door identities derived from placed 3D landing heights, with stable door order
  for equal-height entrances. Horizontal projected entrances no longer invalidate
  a lift, and rotation cannot reverse its fall destination or AI forecast.
  Compiled ladder/wall passages attach the destination receiving plane when they
  change sectors, including short approaches and animation teleports. Binary
  levels retain their existing endpoint and crossing behavior. Precompiled shipping
  containers advance to datadir v23 / mission v14 for the added lift metadata;
  existing JSON descriptors without it retain spatial endpoint selection.

- **Climb landings without mission scripts.** Ladder and wall exit transitions
  use map doors even when no mission VM is loaded. Compiled climb approaches
  carry their landing receiver through a narrow corridor on the lift layer,
  then connect it to the interior receiving plane. Wall approaches use the same
  animation offset as runtime loading. Crenellated transitions retain their
  explicit plane assignment. Together these prevent failed exits and stale
  landing heights on lifts with multiple entrances. Two further library assets
  retain their full movement contour independently of their receiving footprint.
  Clearance repair also covers ladder approaches and wall bottoms, keeping the
  actor's collision box inside its movement area after rotation. Wall tops retain
  their animation-defined radius. If rounding leaves that position blocked,
  compilation finds the nearest clear direction on the same radius without
  crossing a wall or obstacle. Unrepairable approaches retain an explicit warning.

- **Map interactions without mission scripts.** Gate routes, jump selection,
  lockpicking validation, door hover/overlays, patch clicks, building AI door lists,
  blocked-door corpse handling and lift fall destinations use loaded map data
  even when no mission script VM is present. Patch animations follow map switch
  state, and available campaign reinforcements can use map entrances.

- **Stairs that connect after placement.** Compiled stairs generate receiving
  bonds just inside each passage on its owning lift layer, including landings
  whose contours overlap or have gaps. Ground-facing boundaries within the
  passage are clipped to prevent a second receiver switch. Independent lifts
  receive separate navigation layers so overlapping stairs cannot block each
  other. Native compilation adjusts cramped approach points within the owning
  area, preferring the authored direction and then nearby forward/sideways
  clearance. If nothing fits within 64 map units, it keeps the authored point
  and warns. Graph-free pathfinding and approach construction now use the stock
  human profile's 6-by-3 half-diagonal; the former 6-by-4 default incorrectly
  rejected narrow passages. One-time recovery now retains the full movement
  contour of a static lift with one receiving owner, independently of its
  receiving footprint. Binary level data is unchanged.

- **Partial navigation cells at map edges.** Maps whose pixel dimensions are not
  multiples of 64 allocate the final partial grid cells and retain their exact
  image bounds. Actors can cross receiving-surface seams in the right and bottom
  strips, where elevation callbacks previously stopped outside a truncated grid.

- **Fractional receiving-surface seams.** Generated elevation boundaries retain
  subpixel endpoints and distinguish tiny overlaps from gaps, preventing duplicate
  receiver switches while walking over sloped terrain. Splitting retains distinct
  endpoints even very close to a vertex; ground slivers that collapse to one
  float32 boundary become a direct receiver transition. Actor-tick tests traverse
  curved and rising walkways in both directions through queued pathfinding.
  Integer level files remain readable. Packed datadirs advance to version 22 and
  mission payloads to version 13; regenerate older packed data.

- **Focused routing for graph-free maps.** Editor-generated maps now use a
  deterministic Euclidean A* search for visibility routes, avoiding unnecessary
  expansion across distant parts of the same movement area. Visibility checks
  are skipped when an edge cannot improve the known route, and collision-line
  queries skip geometric intersection tests for empty grid cells. Blocked
  pathfinder corridors stop at the first collision instead of collecting all
  candidate lines. Precomputed navigation graphs retain their existing search path.

- **Asset-owned scenery animation definitions.** Map export transforms local
  billboard anchors and display polylines into native map animations, independently
  of mission characters. Definitions name a sprite/profile and its raster center;
  an optional asset resource directory packages pinned manifests and frame images
  into the mod. Export verifies hashes, frame references and sprite centers, and
  warns when unavailable resources force an animation to be omitted. Shared-bank
  references remain supported. Independently authored banks with the same name
  and different pinned contents receive distinct export names, preserving both
  effects and keeping installed shared banks separate. Identical contents share
  packaged files; missing resources omit only the affected bank. An offline
  standalone-effect authoring helper creates reusable local frames without baking
  sprite artwork into the background; placement, copying and save/reopen are
  covered by compiler tests. Pinned effect-only assets show a verified static sprite
  thumbnail in the palette, with legacy key colors decoded. Published effect
  definitions and live map-viewport previews remain unfinished.
  `pipeline/src/author-scenery-animation-assets.ts` generates a fresh asset
  library from local recipes, validates/copies pinned sprite resources and emits
  importable placement/source records. It does not require a source level.
  Runtime library staging now copies the explicitly declared, hash-pinned sprite
  banks so exports from the hosted editor can read their manifests and frames.
  Export warns when placement folds a drawing boundary or leaves a vertical
  segment that the actor-ordering code cannot handle reliably, retaining the
  animation in best-effort exports. Repeated quantized vertices are removed.

- **Bounded shadow regions for map appearance switches.** Export includes each
  asset's possible shadow footprint using the scene's sun direction and lowest
  geometry. Independent switches no longer require full-map state combinations;
  overlapping shadows still combine correctly within the state-image budget.

- **Initial obstacle activity in map exports.** Unavailable sight-changing controls
  retain their initial state while preserving receiving-plane and material data.
  Obstacle `initial_active` defaults to true when absent. Packed datadirs advance
  to version 21 and mission payloads to version 12; regenerate older packed data.

- **Combat blocks mission victory.** The saved gameplay flag
  `prevent_victory_in_combat` defaults on. Script-reported victory waits until
  every PC has left combat; disabling the flag restores unrestricted victory.
  The Gameplay option “Block Victory During Combat (Next Launch)” applies at
  mission launch and is forced off for parity replay.

- **Terrain-following gate barriers.** Assets can define a state-dependent barrier
  with finite vertical reach. Export intersects it with current terrain, preserves
  holes and keeps separate placed gates independently controllable. Controls can
  attach to nearby terrain; unavailable movement-only controls retain their initial
  barriers with an export warning. The south gatehouse includes a closed barrier
  that works on Wychford's continuous terrain. Native checks cover opening/reset,
  sloped ground and an unaffected floor above.

- **Terrain-aware ordinary passages.** Each endpoint of an ordinary passage can
  attach within an authored local segment, following the current terrain while
  retaining its permissions and transition links. Ambiguous or unreachable
  endpoints warn during best-effort export; cropped destinations are omitted.
  Native tests check approaches from both sides and preserved lock rules.

- **Editor-owned interior connections.** The Assets inspector can link rooms in
  separate placed assets. Links survive movement, saving and editable exports.
  Multiple doors in one asset room remain connected automatically; distinct
  authored rooms remain separate. Copying a building preserves its local rooms
  without copying external links, while copying an entire compound preserves
  links within that compound. Missing room definitions warn during best-effort
  export. Compiler fixtures are checked through native room/gate registration.

- **Terrain-aware mask attachments.** Asset masks can select a receiving layer
  through a finite local segment, allowing small terrain-height changes while
  preserving their pixels and masking boundaries. Ambiguous or unreachable
  attachments warn and are omitted during best-effort export. Cottage assets
  include bounded attachments for reuse on uneven editor terrain.

- **Terrain-aware interior entrances.** Assets can author a finite receiving
  segment for each exterior approach. Compilation moves the approach onto the
  current unblocked terrain surface and rebuilds its gate connection. Building
  geometry and lock rules remain authored; unreachable or ambiguous entrances
  are reported rather than attached to an arbitrary layer.
  A repeatable asset-authoring command applies explicit attachment recipes,
  checks local anchors, preserves unrelated features and updates saved-scene
  pins with rollback snapshots. Fifteen library assets include reviewed rules.
  Physical projection receivers can also select terrain through bounded local
  segments, retaining their own top planes and materials without adding movement
  boundaries. Native tests cover receiving height and character-sized routes.

- **Spline-wall gameplay export.** Calibrated assets supply collision volumes and
  walkable surfaces that follow edited wall paths, repetitions, trims, curved
  bends, elevations and corner towers. Native tests check movement obstruction,
  routes around wall ends and ray collision. Prepared wall strips gain explicit
  barrier definitions; conservative envelopes, unsupported source deformations
  and omitted advanced features are reported in export warnings.

- **Placement-based jump connections.** Asset jump edges can declare gap, height
  and overlap limits instead of exact connection sockets. Export pairs facing
  parallel edges and clips them to a shared span, allowing new arrangements of
  independently placed assets. Existing library sockets remain supported.
  Geometric matching follows the runtime takeoff normal, with an exported fixture
  checked through native animation translation for both travel directions.
  Level ledges now trim blocked flight spans against placed solids in both
  directions, including takeoff, the ordinary jump arc and the direct sword-fighting
  flight. Optional body radius/height account for headroom
  and side clearance. Clearance includes fixed-step airborne motion between
  animation targets and short-flight overshoot before the final landing snap.
  Assisted jumps also check their raised, stationary shoulder departure.
  Unsupported automatic jump shapes warn and are omitted; explicit authored
  connections retain their existing behavior.
  Surface-level jump rules generate reusable ledges and receiving bands from
  polygons, including holes, and support multiple neighbouring destinations.
  Generated ledges reserve character-sized walking clearance, increasing the
  minimum inset and trimming corners and holes so takeoff goals are reachable.
  Unusable candidate edges are omitted with warnings.
  An optional endpoint-adjustment limit aligns slightly skewed roof edges to
  level contours while preserving the roof's slope and clipping receiving bands.
  No saved jump zones or neighbour identities are required for these surfaces.
  Nineteen reviewed library surfaces include these rules; the authoring tool
  stages descriptor-bound edits and updates saved-scene pins with rollback backups.
  The Bridge Square central timber house includes an eaves rule verified with a
  complete moved/rotated copy, retaining both buildings' structural collision.

- **Visible load failures.** Failed in-mission Load and QuickLoad requests show
  an acknowledgeable error dialog with the failure details while retaining the
  current mission, instead of merely logging the error.

- **Early-access save compatibility.** Version 96 is the supported compatibility
  baseline; new saves remain version 97. New fields load with backward-compatible
  defaults; saves briefly written as versions 98 and 99 are accepted too. Future save-version bumps
  require absolute necessity and explicit human confirmation. Binary network
  and replay schemas remain separate from the JSON disk-save version.

- **Cooperative campaigns.** The full-game Multiplayer mission list includes
  Start Campaign for a fresh profile, Continue Campaign using the same latest
  compatible checkpoint as the main-menu Play button, and Load Save for an
  explicit save selection. Old multiplayer saves tagged as local diagnostic
  captures can start a fresh lobby, with new player seats and session authority.
  Failed loads stay in the picker with an acknowledged error message. Compatible
  single-player saves can be continued in
  co-op without resetting their progress. Story
  progression, recruits, money, Sherwood management and mission selection persist
  between missions. The host manages campaign decisions; existing shared story
  confirmations and transition barriers keep participants together. The authored
  team stays intact and extra Robins fill missing player slots. When Robin is
  unavailable to the story team, missing slots copy an available teammate instead.
  Temporary copies never become permanent recruits. Host Save and QuickSave write
  resumable campaign saves; joining players retain local diagnostic captures.
  Resuming preserves the saved world and gameplay rules. The lobby displays
  the saved control mode, enemy-health scaling and team in disabled controls;
  player joins do not rewrite these settings. Compatible save-linked recordings
  continue the original input history using save markers, without embedding a
  snapshot in leaderboard submissions. Missing or incompatible replay history
  produces a playable local recording and a clear submission-unavailable reason.
- **Shared Robin supplies.** Robin copies share one stock of arrows, purses,
  food and other consumables in both campaign and single-mission multiplayer.
  Pickups fill one character's capacity, spending updates every copy's action
  buttons, and surviving copies retain the shared supplies if the original dies.
  Health and coma state remain independent.

- **Shared multiplayer story confirmation.** Story dialogue, briefing pages,
  and debriefings stay open until every player in the lobby has acknowledged
  them. A deliberate host exit tells joining players that the session ended.

- **Exact multiplayer teams.** Local and online hosts choose one to five named
  playable characters independently of player count, including duplicates. A
  dedicated review screen supports replacing and removing each slot and warns
  about characters outside normal campaign starting availability. Custom missions
  with unknown rosters say so explicitly. Shared control can use fewer characters
  than players; exclusive and assigned control require one per player. The chosen
  order is preserved at mission start, and extra characters keep authored script
  handles intact. Duplicate health scaling uses the selected team.

- **Mission failure reports.** Failed menu mission launches automatically queue
  diagnostic reports before displaying the recovery dialog. Desktop reports
  include recent logs and mission, multiplayer role, and datadir context; browser
  reports include recent logs. Upload failures retain reports for retry.

- **Spellforge named mission records.** Mission loading and team-requirement
  scans accept SCOT version 5 and the companion named actor, item, scroll,
  location, and patrol formats. Lua name lookups bind to the created entities
  and authored location/patrol slots. Custom mission archives take precedence
  over already loaded bundled levels with the same mission basename. Packaged
  data uses datadir schema 20 / mission schema 11 and must be regenerated.

- **Minimal editor missions.** The Mission tab adds player spawn points and NPC
  soldiers with placement, facing and profile controls. Its character palette
  displays actual library sprites and uses canonical profile identities. A PC/NPC
  category dropdown filters the palette; dragging a character onto the map adds
  it, with a live surface-positioned preview during the drag. Dropping commits one
  placement; leaving the map or cancelling removes the preview. Default names
  follow the character type, while custom names survive profile changes.
  A mission-element list below the palette selects existing placements, whose
  character profile can be changed in the inspector. Placed
  characters show their directional sprites; numeric controls use the editor’s
  drag sliders. The character library includes every PC and soldier’s idle pose.
  Palette previews and new placements face down by default, and soldier labels
  use English names. Characters remain visible across inspector tabs, controlled
  by the Mission tab’s visibility checkbox. Dragging a character moves it at its
  current height with one undo entry; Escape cancels the move.
  Right-drag camera rotation remains available in Mission mode.
  Loading a game-data mission imports PC spawn slots and soldiers into these
  editable lists. Source receiving planes resolve world height once; source
  profile ordering is mapped to library character identities. Campaign-selected
  slots retain no fixed profile and show a blue outline until assigned a character.
  Import is undoable and marks the scene changed. Unsupported entities remain
  previews only, and stored import warnings describe omitted behaviors.
  Mission authoring stays
  separate from map assets and preview population. Export resolves its markers
  against the compiled navigation and receiving surfaces, writes PCs as spawn
  points and NPCs as soldiers, and preserves the editable mission in the embedded
  editor document. Invalid placements are reported and omitted in best-effort
  export; scenes without authored mission markers remain unpopulated.

- **Unified player placements.** New level exports use `spawn_points`, including
  an explicit empty array for maps without PCs. Fixed-profile and campaign-selected
  slots can coexist. Older `spawn`/`spawn_player` fields remain accepted when reading
  older maps, but are no longer written; bundled map descriptors use the new format.

- **Best-effort editor map exports.** Published draft gameplay definitions retain
  explicit issues. Editor export includes available geometry and reports omitted
  unsupported features or disconnected connections in the ZIP's compile report;
  strict compilation remains available for validation. The editable scene remains
  embedded, including mission previews excluded from runtime map gameplay.
  Export progress separates worker-based compilation and packaging from tiled
  image rendering, with cancellation between rendering steps.

- **Asset contact switches.** Explicit asset-local transition anchors join matching
  placed parts into one map switch, combining their gameplay and appearance changes.
  Moving the contacts apart creates independent switches; duplicated assemblies
  remain independent. Compilation rejects incompatible trigger and door rules.
  One-time gameplay recovery also restores local appearance bindings from exported
  preview provenance, requiring a unique recovered switch on the owning asset.
  Mission names and unresolved cross-asset ownership remain explicit errors.
  The baker resolves initial/applied model views to their pinned primary switch;
  duplicated placements retain independent switches even with identical preview names.
  Inserting a gameplay-enabled endpoint asset loads and saves both model variants
  as one placement, including their resource pins and independent state controller.

- **Independent light receivers on shared height planes.** Map compilation separates
  navigation layers when an anchored light would spill onto an unrelated receiver.
  Flat and elevated recovered fields retain local receiver anchors, full contours
  and ambience filters; all dependent runtime references are rebuilt after placement.
  Finite asset-local receiving segments can attach a light to a sloped surface
  after placement, rejecting absent or ambiguous receivers instead of choosing a
  floor by an unrestricted height search.

- **Independent environmental light-region assets.** Offline authoring can create
  invisible, movable lighting fields with local contours, receiving anchors and
  ambience filters. Explicit ownership declarations pin their source records;
  map compilation consumes only the placed asset definitions. See the
  [map data checklist](../level-editor/docs/map-data-checklist.md).

- **Terrain authoring in the level editor.** The Draw tab creates continuous
  rectangular ground regions with grass, dirt or water and an elevation in one
  panel. Select ground in the scene, resize it with corner handles, or move it
  and change its elevation using the standard asset gizmo. Terrain, path, wall
  and export numeric fields share draggable number controls; each gesture
  commits one undoable edit and Escape cancels its preview. Exact terrain bounds
  remain under Position and size. Later regions replace earlier ground, including
  lower river beds. Paths
  and rivers can set a whole-path elevation; their curved footprints carve the
  ground consistently in the viewport and export. New placements use the terrain
  height and compensate for elevated local asset bases. Export derives navigation
  layers and connected areas, excludes water, retains asset floor/door ownership
  and joins explicit exterior navigation sockets to surrounding terrain. Terrain
  is saved with the map, rendered into mod ZIPs and included in camera framing.
  In-editor instructions explain how to test an exported ZIP in Custom Missions.
  Terrain regions are flat; road/river export currently requires a uniform
  elevation. Walls and scripted mission authoring retain their existing limits.

- **Physical receivers sharing navigation.** Asset gameplay can bind a physical
  receiver to an existing navigation area through an asset-local anchor. Sloped
  elevation and physical geometry remain independent of the movement boundary;
  placement resolves the binding afresh without source-map indices. Missing,
  blocked or ambiguous anchors fail export.

- **Asset-owned obstacle query order.** Gameplay definitions can assign query
  precedence to physical part and volume IDs. Compilation preserves this order
  after placement and volume assembly, rebuilding mask and sight-state indices.
  Equal priorities retain placement order; incompatible priorities on joined
  volumes fail. Offline recovery extracts precedence into assets, so export
  requires no source-level lookup.

- **Explicit visual-component collision ownership.** Asset parts can retain their
  visual bounds and provenance while disabling physical collision. Offline
  `author-owned-volume.ts` assigns a reviewed unlinked volume to one asset frame,
  verifies source/model/descriptor pins and accounts for every visual component.
  Compilation rejects obstacle links to disabled parts and uses only the resulting
  asset definitions. Models and independent traversal geometry remain intact.

- **Reviewed physical-volume partitions.** Offline asset authoring can divide a
  constant-height volume along explicit asset seams while checking coverage and
  overlap. Source, model and descriptor hashes guard the extraction. Each piece
  becomes local collision in its own asset; map compilation needs no source-level
  lookup. Explicit local seam edges let matching flat pieces compile into one
  sight volume, removing artificial internal faces. Moving the pieces apart
  leaves independent volumes. Linked or sloped geometry requires separate authoring.
  Stacked pieces can retain a complete ordered footprint with explicit height
  ranges and top/bottom cap seams. Matching full faces assemble into one volume;
  detached pieces retain their own heights. Gaps, overlapping height ranges,
  ambiguous matches and incompatible physical flags are rejected.
  Draft staging preserves model/resource paths in an isolated library overlay,
  checks input hashes and scene overrides, and reopens the resulting editor scene.

- **Independent physical asset drafts.** Offline authoring can restore a reviewed
  missing object as a separate editor asset with local collision and a volume
  preview mesh. Source hashes and ownership recipes are checked before extraction;
  compilation then uses the asset alone. Drafts explicitly retain unfinished
  appearance and mask status instead of assigning missing props to nearby buildings.

- **Mask-controlled depth baking.** Asset gameplay can declare
  `maskOcclusionNodes` for parts whose complete occlusion is authored by typed
  masks. Export keeps their color geometry while baking the depth of surfaces
  behind them, so static mesh depth cannot override mask deactivation. Other
  parts retain depth occlusion. Existing assets require complete mask authoring
  before opting in; visual-state color resources remain separate work.

- **Asset-local interior connections.** Independently placed buildings can share
  a virtual room through matching passage sockets. Sockets carry local positions
  and opposing directions; moving a building or connector away separates the
  rooms. Doorless passage assets can connect several entrances, and disconnected
  passages create no empty runtime building. Export rebuilds door registrations
  and transition references after joining, including rotated and duplicated
  assemblies. No occupants or mission content are introduced.

- **Asset-local door state links.** Map transitions can change ordinary/interior
  door permissions or be activated by a door. Export regenerates native door
  indices for each placement and rejects missing or conflicting links. Door-only
  transitions require no mission content. Recovery retains unresolved ownership
  and state geometry as explicit authoring gaps. Unambiguous linked state geometry
  provides recorded ownership evidence. Ordinary passages can meet stair/lift
  surfaces without being converted into lift doors. Offline door ownership
  declarations can attach empty gate passages to validated asset frames; source
  indices are replaced by local endpoint definitions before compilation.

- **Terrain beneath changing obstacles.** Offline recovery separates permanent
  ground from state-dependent exclusions and retains every changing contour and
  patch association for asset authoring. Static diagnostics count omitted
  transitions explicitly. Fixed-point ground operations and rounded-boundary
  normalization handle touching rings without discarding entire terrain areas.

- **Asset-local jumps.** Map export retains 3D paired edges, receiving regions,
  long-jump flags and helper requirements, rebuilding native jump zones and gates
  after placement. Source extraction preserves edge elevation and repairs cropped
  zone references. Offline recovery keeps ambiguous ownership and unresolved
  landing geometry visible. Local sockets pair edges across independently placed
  assets. Unmatched edges leave that connection unavailable with an export warning;
  restoring the placement reconnects it. Export removes unused landing zones and
  rebuilds references for remaining pairs, while ambiguous matches and conflicting
  traversal rules remain errors.

- **Asset-local gameplay lighting.** Map export transforms planar light/shadow
  contours, resolves their receiving layer and preserves ambience masks. The same
  compiled map supports different mission ambiences without shifting interior
  links. Offline recovery can attach a region spanning several parts of one
  asset, requiring complete footprint coverage without unowned interior gaps.
  Raised terrain on layer zero retains its elevation across non-walkable
  receiving-footprint notches. Recovery reports ambiguous ownership and regions spanning
  multiple receiving planes; these remain authoring gaps.

- **Compiled map mask interchange.** Native map descriptors accept typed mask
  bitmaps, character/projectile polylines and sight-obstacle links. Mask-only
  transitions rebuild per-layer references and switch initial/applied masks.
  Invalid bitmap rows and conflicting state ownership fail before construction.
  The editor has a binary-silhouette encoder verified against the native decoder.
  Assets can author local coverage triangles, receiving anchors, masking
  boundaries and obstacle/state links. Compilation rasterizes placed coverage in
  bounded tiles, preserves holes and concave boundary steps, and rebuilds links
  independently for rotated/duplicated assets. Map ZIPs include the resulting
  masks. Recovery for existing assets, textured-mesh extraction and coordinated
  visual/depth-state export remain unfinished.
  A one-time recovery audit strictly decodes source masks and verifies exact
  coverage after merging pixels into nonoverlapping rectangles. These remain
  intermediate screen-space authoring data until mapped onto an owning asset's
  surfaces; the runtime compiler does not read the source maps.
  Surface lifting can intersect that coverage with explicitly supplied owner
  triangles, resolve overlapping depths and produce local geometry while
  rejecting uncovered pixels. Batch ownership and migration remain unfinished.
  Published static opaque mesh parts can supply those triangles, including
  nested transforms. Recovery verifies the clipped result through the compiler's
  rasterizer; transparent materials require explicit texture coverage extraction.
  Character and projectile boundaries can independently use open polylines,
  preserving concavities and endpoint steps without adding a closing segment.
  One-time rule recovery combines verified coverage, explicit boundary heights
  and local obstacle ownership into a complete asset mask definition. Its output
  recompiles independently of source maps; existing-map migration remains pending.
  Patch mask references are validated as layer-local pairs and can be recovered
  into local state IDs after ownership is established, with independent compiled
  references for duplicated assets.
  Ordinary navigation regions can join across separately placed assets through
  explicit local 3D boundary edges. Coincident opposing edges share one movement
  region while retaining their receiving planes and materials; detached edges
  remain independent. Validation rejects ambiguous or overlapping connections.
  Rotated/duplicated assemblies and native reachability across the resulting
  multi-plane region are tested. Recovered-map join authoring and publication
  remain unfinished.
  Reviewed join recipes can now be applied by the one-time asset migration with
  source/model pins, source-region ownership checks and complete seam validation.
  Lincoln's north curtain pair reproduces its baseline exactly and retains native
  construction after either asset is independently moved; its definitions remain
  unpublished while the map's other parity gaps are resolved.
  Nottingham has reviewed definitions for four seams across seven assets, including
  a sloped wall pair. A repeatable placement verifier checks recovered definitions,
  pins, baseline assembly and independent moves, preserving failures in its report.
  Six Nottingham owners pass westward movement and native construction; one still
  invalidates a neighboring stair connection. Full-map connectivity remains open.
  York has three reviewed groups across six assets, preserving baseline geometry
  and bindings after reference remapping. Five independent moves construct
  natively; moving the causeway invalidates a neighboring slope connection.
  Seven additional candidate groups still need topology and projection review.
  A projection-coverage comparator distinguishes equivalent polygon subdivisions
  from changed receiving geometry, exact height planes, flags or material rules,
  independently of rebuilt motion/material indices. It identifies York's inner
  curtain subdivision as coverage-equivalent while retaining the terrace's material
  difference. Overlapping-plane priority and traversal require separate checks.
  A native receiving-query probe additionally compares sampled coverage, float32
  elevation and material selection. York's inner curtain candidate passes coverage
  and material checks but changes elevation by one float32 step at 28,268 sampled
  points; its baseline self-comparison passes. This candidate remains unreviewed
  until receiving-plane arithmetic and traversal are verified.
  Receiving materials now carry optional ordered local plane anchors through
  recovery, placement and clipping into native height calculations. Regenerating
  the York pair removes every sampled height difference, with no query tolerance.
  A small difference in which authored plane owns coverage still needs review.
  Shipping datadir 19 / mission 10 preserve these anchors; older binary shipping
  bundles require regeneration. Existing hackable JSON remains compatible.
  All nine recovered static maps construct natively with the new anchors. An
  ordered float32 anchor audit finds 698 exact source matches and separately
  reports 413 fallback receivers without explicit anchors. Those receivers still
  need coverage/material ownership review; these checks do not certify full parity.
  The compiler now preserves gaps between explicit receiving supports instead of
  filling them with default material. Recompiling all nine maps removes 412 such
  receivers while preserving anchored receivers and other geometry after interior
  constructor reference remapping. Native
  platform queries verify the opening remains empty. Derby's explicitly authored
  second drawbridge's remaining receiver was subsequently traced to editor preview
  bounds. Recovery no longer invents a walkable surface from that placeholder, and
  compilation excludes preview bounds from automatic collision. Explicitly authored
  surfaces and volumes still work; the bridge's actual state behavior remains open.
  A patch-dependency audit now identifies shared sight/mask/door references and
  receiving-surface state changes without importing mission actors or scripts.
  The retained mission inventory exposes a shared Derby bridge obstacle and
  Leicester's changing projection surfaces; ownership and projection-state
  compilation remain unfinished.
  Native interchange accepts projection obstacles in sight-state transitions.
  Activation, swap and reset tests verify changing collision with stable receiving
  height/material lookup, which includes inactive projection surfaces. Asset surfaces
  can link to explicit local receiving volumes, preserving thickness, physical flags,
  material references and sight-state bindings through placement and duplication.
  The exported fixture verifies top/underside collision and opaque-ray blocking in
  the native runtime. Receiving links can also reuse physical asset parts. One-time
  recovery restores Leicester's three map-patch receivers with exact ordered
  float32 geometry and flags; its five recovered transitions pass native apply/reset
  checks. Publication, shared controllers, state visuals and traversal remain open.
  Physical receivers use their first three ordered vertices for the receiving
  plane while preserving later vertex heights. A broader static audit constructs
  Croisement03 with 14 physical receiver links; other candidate maps still need
  navigation-region and material-priority fixes before migration.
  Receiver ownership now excludes navigation holes and blockers when assigning
  areas, preventing a surrounding platform from supplying an island's materials.
  The exported island fixture verifies native material/height queries and the
  disconnected walking route. All nine static diagnostics construct after removal
  of 96 receiver records incorrectly assigned across disconnected areas.
  Reviewed physical-receiver recipes now migrate through `--projection-definitions`,
  checking source/model pins and exact geometry before atomically creating local
  links. Croisement03's 14-link recipe reproduces the audited baseline; the baseline
  and 13 independently moved asset cases construct natively. Publication and full
  gameplay verification remain incomplete.
  Navigation-only state assets can use a reviewed plane to place changing contours
  outside receiving coverage without inventing a receiver. A pinned Croisement03
  recipe restores its upper-terrace boundary; eight recovered transitions pass
  native apply/reset checks, leaving one multi-asset movement group unresolved.
  Staging also retains inherited ownership catalogs without symlink write collisions.
  A reviewed Croisement03 assembly groups the four southwest obstacle pieces that
  change together. All nine map movement groups now recover; native apply/reset
  checks pass before and after moving the assembly, with exact physical geometry
  and flags retained. Its three state-controlled masks and visual resources remain
  unrecovered, so this is not yet full patch parity.
  Reviewed static-mask recipes now run in the asset migration with pinned source
  and model hashes. One Croisement01, seven Derby, sixteen Leicester, four Lincoln,
  sixteen Nottingham and twenty York
  masks have exact baseline coverage and boundary round-trips with native
  registration and independent placement checks. Jump landing anchors use their
  owning asset's receiving footprints and evaluate height at the integer movement
  point, preventing a moved neighbor from invalidating an unrelated landing zone.
  Mask receiving layers remain resolvable when movement collision covers their
  anchors, provided authored surface support identifies one surviving layer.
  Door and jump destinations continue to require walkable receiving positions.
  A repeatable mask-recovery verifier checks source/model pins, exact coverage and
  rules, independent translations and native diagnostic descriptors. Failed runs
  invalidate earlier success manifests. Fractional receiving anchors retain their
  authored position for slope elevation checks.
  Verification supports masks split into bitmap tiles, checking their exact
  combined coverage and consistent bindings while rejecting unaccounted records.
  Full-map recovery and publication remain incomplete.
  A surface-support audit reports missing pixels, interior gaps and repair bounds
  for reviewed mask sets, including whether both sides of a state change have
  full pixel support. Migration rejects unsupported coverage before surface clipping.
  Light-region migration supports splitting across receiving planes while preserving
  holes and ambience filters. It rejects splits whose integer output changes any
  receiving contour, and records source-to-local region IDs in the recovery report.
  Asset-local light receiving anchors can instead preserve one exact contour across
  several elevations. The compiler resolves their layers after placement and emits
  the contour once per layer, retaining ambience filters without fractional cuts.
  Unrestricted non-clickable passages can opt into continuous navigation: when
  placement joins their two areas, compilation omits the redundant gate and
  rebuilds remaining door bindings. Restricted and state-linked doors remain required.
  Reviewed ambient regions can be authored as standalone non-rendering assets with
  local sound geometry and pinned editor placements. Derby's north and west zones
  retain exact sound definitions and move independently without changing navigation.

- **Preserved movement boundaries.** A labelled ordinary asset surface can retain
  its outer contour separately from crossing movement obstacles. This preserves
  thin walkable strips whose intersections fall between integer coordinates;
  compiler-export and native navigation tests cover the behavior. Enclosed islands
  are partitioned with coverage checks; recovery can split nearly touching
  fractional holes before final assembly. Joined pieces remain unsupported in
  this mode. Offline recovery exposes `--preserve-ground-boundaries` for comparison
  drafts, without changing default recovery. Full-map parity and publication
  remain incomplete.
  Shared contour labels let separately placed asset fragments reconstruct one
  exclusion while retaining distinct overlapping contours. Exported native tests
  verify the fractional intersections without rounding them into new vertices.

- **Asset-local movement transitions.** Map compilation clips initial/applied
  blockers to their placed navigation areas and assigns independent state bits
  to each instance. Blocker holes, trigger polygons and local sight-obstacle
  references are retained. The game
  applies and resets all affected areas through native map patches, without
  requiring mission actors. Explicit non-rendering asset frames support reusable
  navigation-only boundaries in the editor, including save/reopen and independent
  placement. Their reference points can resolve inside static collision while
  traversal endpoints still require clear positions. An asset can explicitly select
  permanent part/volume solids while authoring independent changing contours;
  permanent geometry retains its local movement clearances. Visual and mask transitions remain unfinished;
  this does not certify extracted-map parity.

- **Map selection and safe editing.** The editor opens on thumbnail cards, with
  New map on that screen and a close control in the workspace. Unsaved edits warn
  before leaving; loading uses a cancellable modal. Local copies can be renamed
  or deleted, and saves from built-in maps allocate distinct modified copies.
  Native browser thumbnails accompany saves. The header features Robin's original
  transparent animations, aligned by sprite hotspots, with idle gestures and
  transitions into and out of his hover dance.

- **Asset-local gameplay compilation.** The editor builds planar walkable regions with slopes and holes,
  collision/sight geometry, passage/gate connections, lifts, empty building
  interiors from pinned
  asset definitions and placement transforms. Explicit movement contours can
  differ from sight footprints and follow asset placement, preserving courtyard
  holes. Asset-local movement clearances open only their owner's derived collision
  on the matching plane, without removing sight geometry or other assets' blockers.
  Local material polygons follow placement and rebuild separate ground and
  obstacle lookup references. An empty ground list leaves obstacle-only materials
  inactive on the ground; material allocation also preserves interior door links.
  Walkable surfaces can author receiving-material defaults, local region references,
  footprints and overlap priority. Compilation partitions receiving faces independently
  of navigation, preserving joined walking areas across material boundaries.
  Terrain definitions supply forest behaviour and default material; conflicting
  defaults from multiple terrain assets stop export.
  Environmental emitters retain asset-local geometry, delays, volume falloff,
  noise-covering ranges and ambience filters; shared audio sample references use
  the base installation. Recovery reports ambiguous emitter ownership explicitly.
  Light/shadow contours resolve against ordinary surfaces and sloped traversal
  areas, retaining mission ambience filters. Recovery allows exterior contour
  extensions only when they do not cross uncovered potentially walkable geometry.
  Native patch bindings can update multiple navigation areas together, retaining
  movement-sector and line activation through apply/reset. Combined visual,
  sight and mask state compilation remains unfinished.
  Offline terrain recovery reports reconstruction error on the integer
  movement grid and keeps unresolved ownership visible. Recovery drafts are
  checked against the asset schema; passage connections can omit click polygons,
  and doors retain alternate lock rules. Lift endpoints are selected spatially,
  including lifts whose two doors use the same action type. Geometry-only assets
  retain part-derived collision during recovery; standalone passages receive
  independent ownership. Separate full-scene and static-geometry diagnostics
  expose remaining authoring gaps. Missing metadata stops export;
  the compiler has no source-level input. Runtime tests construct navigation and
  door links without mounting a game datadir. Full extracted-map parity remains
  unfinished. Every ZIP also contains the editable level JSON, including unsaved
  edits, for reopening with its pinned asset library.

- **Level-editor mod compilation.** Export mod ZIP renders the current placed
  scene into a hackable datadir archive with a map PNG, minimap, 16-bit sprite
  occlusion depth, and a geometry descriptor. Compilation
  respects hidden placements, transforms and export crops. Maps contain no
  player spawn; characters and their starting positions belong to missions. Mission scripts,
  interactive patch transitions and preview population are not compiled.
  See [map compilation](../level-editor/docs/map-compilation.md).

- **Browser-local map authoring.** Published assets load automatically over HTTP
  from the existing library index, with source-map folders keeping the catalog
  organized. Save writes map copies to OPFS; Download exports the current JSON.
  The open map name is followed by readable, map-filtered mission choices. Reset
  view restores the map camera, and framing remains available through `f`.
  See [editor storage and controls](../level-editor/docs/3d-editor.md).

- **Focused level-editor workspace.** A compact map picker replaces the map
  button strip. Selection, drawing, and view settings have separate inspector
  sections; the asset browser can be resized or collapsed to a narrow title strip.
  Its arrow remains available when collapsed, and wider panels show more columns. A status
  bar tracks unsaved edits, and contextual help explains navigation shortcuts.
  Unfinished paths keep drawing controls visible until finished or cancelled.
  Asset drags render provisional instances in the scene; returning to the library
  removes them, and releasing commits one insertion. Transform fields support
  horizontal dragging with one undo step and Shift for finer control. Repeated
  normal clicks keep group selection; Alt-click selects individual parts.
  New maps need only a name and persist an unbounded canvas (`size: null`);
  the adaptive workspace grid is an editor guide, not saved terrain or a boundary.
  Assets added from the library are placed at the viewport center. Fixed image
  extents are deferred to compilation. An optional, undoable export frame can
  be set later in View, adjusted numerically, fitted to content on demand, or
  removed. The frame is advisory in the viewport and does not clip editing;
  compilation must honor the chosen crop even when assets cross its edges. TODO: the reconstruction-only game baker
  still cannot compile authored maps, imported assets, or spline geometry; an
  authored compiler must crop their actual placed geometry and rebase game data.

- **Visible-surface editor picking.** Selection and drag rays cover the complete
  orthographic clipping range with reversed depth, skip transparent foliage and
  atlas pixels, and
  identify imported parts by their actual wrapper objects. Reused model node
  names no longer redirect clicks to unrelated assets. Building texture ownership
  masks retain their opaque display behavior when picking.
- **Wychford terrain and courtyard revision.** Reference-guided OpenRouter
  Sunburst generation produces a 5120-square terrain mosaic from four overlapping
  patches. The castle has worn earth and paving, its gate faces outward, and the
  repositioned keep leaves more space along the eastern curtain. Full-resolution
  ground art is retained in scene export.


- **Authored town population previews.** Wychford includes 46 soldiers, civilians
  and beggars, 14 item placements and nine guarded or civilian routines inspired
  by the mission layouts. Directional idle/walk sprites load from library atlases;
  patrols pause and reverse, paired guards use separate lanes, and wall sentries
  stand on the curtain walk. The editor exposes pause and route overlays plus
  actor duties and beggar hints. Population data saves with the scene. Combat,
  dialogue and item collection still require a playable mission export.


- **Background replay checkpoints.** Leaderboard viewers download and validate seek
  checkpoints after the mission starts. Interactive seeks restore a settled
  checkpoint first, then render progress while simulating the remaining records;
  a sidecar arriving during a seek becomes usable without restarting playback.

A list of which additional features we have added, which ones we might still want to add, and which old ones we will NOT add.

## Done

- **Wall corner presets and editable footpaths.** Wall paths can pair a pinned
  curtain asset with a tower model. Turns above a configurable angle create
  oriented corner towers automatically, with individual opt-outs and scale/
  rotation controls. Named presets persist across levels in the same browser;
  each map retains its own settings and source hashes. Footpaths are separate
  textured, height-aware splines with save/reload and undo support. Wychford now
  places its village west of the river and stronghold, using Leicester, Derby
  and Sherwood props, terrain and selected irregular trees.

- **Editor sun and terrain shadows.** Authored maps can save a sun direction,
  elevation and shadow strength. Editable meshes cast filtered shadows onto
  terrain without relighting baked source textures. River surfaces and editing
  handles do not cast shadows. The wall spline editor can flip its cross-section
  to place a one-sided parapet on the exterior of a fortification.

- **3D spline authoring and Wychford.** Draw and reshape river ribbons and
  battlement walls with viewport control points. River tiles repeat by arc
  length; wall meshes are subdivided and bent in 3D with interpolated UVs.
  Width, repeat length, closed paths, source alignment and end trimming are
  editable. Paths participate in save/reload and undo/redo. The Wychford scene
  generator assembles an original riverside market town from shared assets,
  with editable river and bailey-wall paths. These are editor geometry;
  collision/navigation and game baking are not generated for splines yet.

- **Shared 3D editor asset library.** Browse published assets from every source
  level in one panel, with 3D previews that rotate on hover, tags, text search,
  and combined asset-type/source-level filters. Drag objects onto the scene's
  terrain or use Add to scene. Cross-level instances preserve their source
  provenance through save/reload and undo/redo. Catalog entries can supply
  `asset_type` and `tags`; older entries use name-based type classification.
  Background artwork is previewable but remains non-insertable. Imported assets
  can be saved in editor documents; game baking remains unsupported.

- **Local directional gamepad movement.** Left-stick and optional WASD movement
  drive each selected hero on their current layer, stopping at blocking geometry
  instead of pathfinding toward a projected cursor target. Walk/run input remains
  available. Releasing input, changing selection, or disconnecting stops direct
  movement; point-and-click orders retain route finding.

- **Hackable scenery-occlusion PNGs.** Datadir conversion exports every level's
  authored occlusion bitmaps as lossless grayscale PNGs with map placement,
  global/per-layer identities and obstacle references. Existing hackable datadirs
  can add these sidecars using `convert_datadir --mask-pngs-only` without rewriting
  level JSON or converting other assets; see [modding tools](MODDING_TOOLS.md).

- **3D editor mission previews and perspective.** Load a mission from the connected
  hackable datadir to display its initial characters with all 16 camera-relative
  directions, bonus/scroll sprites, targets, and mobile objects. A perspective slider
  adds distance scaling while smoothly preserving average projected map scale, with tight clip
  planes and reversed depth where supported. Separate upright, prone, pickup, and
  scenery profiles adjust with camera height. Prone bodies use fixed source-view
  angles; other directional sprites face the camera. Coats of arms use cylinders.
  Perspective panning follows the floor without changing camera height or zoom.
  Orbit preserves pivot distance; wheel zoom dollies freely without a map-fit limit.
  An optional rotation lock snaps camera headings to the 16 sprite-view angles.
  A separate sprite-orientation toggle selects camera-facing or fixed projections.
  Authored sprite shadows project onto support planes with ambiance-specific
  darkening. Explicit target heights are respected. Hackable conversion includes
  shared pickup/relic sprite banks. Previews do not execute scripts or alter missions.

- **Named level-editor assets and Blender refinement exports.** Derby's generated
  parts belong to named building and wall groups, with second-click part selection.
  Imported maps can also supply group and part names through their GLB metadata,
  preserving authored ownership without replacing user-edited groups.
  Reusable Blender scripts preserve source IDs while exporting refined maps and
  standalone asset models. Separate covered and revealed projection sources retain
  interior artwork and patch-state metadata.

- **Co-op for up to five players.** The Multiplayer lobby supports online games
  and local controller sessions. Press A on each controller to join; the keyboard
  player can be toggled in the lobby. Choose a mission, Shared/Exclusive/Assigned
  character control, initial assignments, and which mission hero to copy into
  missing player slots. Shared control allows everyone to select any hero;
  Exclusive prevents simultaneous ownership; Assigned reserves each chosen hero.
  Extra copies increase hostile soldiers' health by a configurable percentage
  (including reinforcements). A required hero is lost only when every copy dies.
  Robin copies share consumables; other hero copies retain separate inventories.
  Copies have independent health and coma state and do not become permanent
  campaign recruits.
- **Fair preserved-lives accounting.** By default, hostile soldiers already
  dead after mission startup setup are excluded from the campaign preserved-lives
  percentage and post-mission recruitment calculation. The Gameplay option
  “Exclude Starting Corpses” can be disabled to restore the original counting
  behavior. A separate default-on “Exclude Required Kills” option also omits
  Guisbourne in `S05_Yrk_EC`, Longchamps in `H10_Yor_VL`, and the Sheriff in
  `H12_Not_MP`, whose deaths are required for victory. Optional kills (including the Derby generals) still
  count. Disable both options for the unadjusted totals. Parity sessions disable
  both. These rules affect future mission results; existing campaign totals are
  not recalculated. See [the mission audit](slop/PRESERVED_LIVES_MISSION_AUDIT.md).
  Mission results and main-menu campaign stats show both the percentage and
  “saved N of M”, using the same eligible population. Older profiles that stored
  only a percentage show “count unavailable” until campaign synchronization
  restores their exact counts.
  Completing the campaign with at least seven royal relics now continues into
  the Sherwood bonus ending after the final cinematic, with the named companions.
  The relic objectives can be completed nonlethally; Scathlock's optional blazon
  is not required to unlock this ending.
- **Dynamic local cameras.** Nearby players share a view. Distant groups get
  separate cameras with rotating boundaries that disappear as players regroup.
  Up to five controller cursors remain in their own views. Reconnecting a
  controller with A reclaims a disconnected local slot. Local and online
  players use the same deterministic seat/command model; mixed sessions are
  not exposed by the lobby.
- **Chat and connection status.** Enter opens chat; developer commands require
  a leading slash (for example `/HELP`). Recent messages and connection notices
  remain visible for seven seconds with chat closed. The player list shows
  measured round-trip latency along the session route to each online player.
- **Optional WASD direct movement.** Gameplay settings offer keyboard direct
  control in both solo and co-op: WASD walks, Shift runs. Point-and-click remains
  available. Controllers retain left-stick movement, right-stick aiming/combat
  gestures, shoulder-button hero cycling, and face-button actions. A/B/X
  (Cross/Circle/Square) select the first/second/third ability respectively.

- **Responsive browser replay seeking.** Timeline fast-forward runs in adaptive batches targeting two progress updates per second during seeks. Normal playback keeps its existing frame rate. Progress remains visible, and a new timeline target redirects an in-progress seek.

- **Regular native presentation samples.** Camera and entity interpolation share
  a monitor-rate sample clock across simulation ticks and intermediate renders.
  Frames wait for their slot instead of submitting bursts when swapchain images
  are immediately available; long stalls skip expired slots. Platforms without
  a reported monitor rate retain their existing presentation pacing.

- **Vulkan presentation feedback profiling (Linux).** With
  `ROBIN_GAMEPLAY_PROFILE=1`, supported Vulkan devices collect asynchronous
  `VK_EXT_present_timing` feedback for actual display presentation. INFO logs
  report measured display gaps, missed fixed-refresh slots when known, and
  120-interval summaries; add
  `RUST_LOG=info,presentation_perf=debug` for per-present IDs, display-stage
  timestamps, GPU-ready timestamps and time-domain identifiers. The selected
  stage is explicitly logged (`first_pixel_out` or `first_pixel_visible`).
  Unknown timestamps are not classified as discarded frames; VRR/unknown
  refresh timing does not produce missed-refresh counts. Resize resets tracking.
  Unsupported devices retain CPU presentation diagnostics, which are estimates.
  Local wgpu-hal hook provenance is in `ROBIN_PATCHES.md` of the
  `phiresky/wgpu-hal` fork pinned in the root `Cargo.toml`.

- **In-game leaderboard registration.** Every submission checks the uploader's
  server profile first: Previous Plays, mission-end buttons, retries, and
  automatic uploads share one registration gate on native and browser builds.
  New identities get a username prompt with Register and submit and Cancel
  actions. Automatic uploads show the prompt even when mission-end boards are
  hidden. Registration uses the same durable identity and server as the upload;
  success resumes that upload, while cancellation prevents it. Invalid names
  and network errors remain in the prompt for retry.

- **Replay hash upgrades.** `robin --upgrade-replay INPUT --upgraded-replay OUTPUT`
  runs the recording twice on the current engine, compares every frame, and
  atomically publishes a new JSONL replay with current hashes and save markers.
  Published hashes follow the normal 25-frame checkpoint interval; both passes
  still compare every frame, including frames without a published checkpoint.
  Inputs, campaign, settings and existing eligibility taints are preserved;
  no migration-only taint is added. The server still verifies the resulting
  replay and computes its leaderboard metrics. The native API is
  `replay_upgrade::upgrade_replay`. JSONL recordings and mission archives from
  schemas 43–47 have compatible inputs; current compact recordings also work.
  Other input schemas and obsolete embedded saves require separate migrations.
  Set `ROBINHOOD_DATA_DIR` to the matching game installation. The source and
  any existing output are never overwritten.

- **Replay compatibility across commits.** The verifier accepts any replay
  whose replay schema and network protocol versions match its own, without
  matching client source commits or save-format versions. Upload,
  verification, download, and playback preserve the recording's source
  metadata. Compact replay links use the current browser runtime. Signatures,
  resource bounds, and deterministic replay checks remain enforced.

- **Replay-based leaderboard submissions.** Mission-end screens and Previous
  Plays upload the recording itself. The client fetches the published board
  list, picks the one board for the installed edition (Demo or Full) that
  lists the mission and whose simulation policy admits the recorded gameplay
  configuration (an exact preset board wins over an any-configuration board;
  no match or an ambiguous match shows the leaderboard as unavailable),
  signs a timestamped `SubmissionV2` (board, mission, replay SHA-256 and
  length) with the player's identity key and posts it with the canonical
  compact replay in one request; there is no server-issued challenge. The server
  re-simulates the replay against its raw game content, checks the recorded
  state hashes and outcome, and scores it. No pre-game grant, content or
  ruleset manifest, or multiplayer co-signing is involved: the uploader signs
  alone, and anonymous player counts come from recorded seat events. Durable
  submission links use the recording's content identity, which survives
  uploader rebuilds. Campaign Manager and Hall of Deeds link to mission
  boards.

- **Bundled modding tools.** Native release packages include `cpf_to_json`,
  `encode_mod_sprites`, `disasm_scb`, and `dump_res` from the new
  `robin_modding_tools` crate. Rust command-line tools use Clap derive for
  consistent help and argument validation. See [Modding tools](MODDING_TOOLS.md).

- **Generic JSON Patch mods.** RFC 6902 operations edit decoded profiles,
  levels and resource descriptors. Profile filenames provide named keys while
  existing numeric slots are preserved. Patches compose across directory and
  ZIP overlays, with atomic installation and typed error reporting. Legacy
  mod patch formats have been removed. See [JSON Patch mods](JSON_PATCH_MODS.md)
  for filenames, examples, the profile exporter and current limits.
  Profile patches now target the on-disk canonical profile.cpf.json directly;
  named maps and explicit order lists preserve numeric IDs without a separate
  patch view. Regenerate older hackable profile JSON with the CPF exporter.

- **ZIP mod overlays:** Put a mod ZIP directly in `mods/` or the configured
  `ROBINHOOD_MODS_DIR`; `details.json` and `Data/` should be at the archive
  root. A single wrapping folder is also accepted. JSON mission discovery,
  soldier profile patches, PNG characters, standalone VQ characters, and
  shipping VQ families use the same overlay reads as directory installs.
  ZIP directories are enumerated without extraction; in-memory mission
  archives use the same sprite loader. Disposable PNG caches remain optional
  and are written only for directory installs. Install just one edition of
  a mod when editions share mission and character identifiers.

- **VQ sprite mod packages:** Custom `.rhs.d` directories can contain an
  authored `sprites.vq.zst` instead of PNGs and a manifest. The
  `encode_mod_sprites` binary converts a mod using exact four-pixel RGB565
  dictionaries and the shipping adaptive VQ codec, then verifies every frame
  and animation field through the runtime reader. Transparency, shadow keys,
  odd frame widths, and profile stats are preserved. Loading reconstructs the
  existing runtime sprite representation. These packages require this engine
  update; the source PNG mod remains separately editable.
  Build with `cargo build -p robin_modding_tools --bin encode_mod_sprites`, then run
  `target/debug/encode_mod_sprites SOURCE_MOD DESTINATION_MOD`.
  Whole-mod conversion now groups identical animation layouts into
  `Data/Characters/*.sprites.vq.zst` families: one frequency-ranked dictionary
  per character, entropy-selected two-hub colour prediction, temporal/direction
  references for standalone hubs, and the production 1,048,576-tile groups.
  Family files use the shipping bank, grouped encoder, and dependency-aware
  materializer directly. The converter also replaces `.map.png` terrain with
  JXL quality 80 `.map` files through `cjxl` and verifies runtime decoding.

- **Custom mission pane scrolling:** The mission list and wrapped mission details
  scroll independently under the pointer. Both show draggable scrollbars when
  their content overflows; selecting another mission resets its details to the top.
  A shared scroll view also powers Campaign Manager’s Hall of Deeds, Achievements,
  mission details, and Previous Plays, with a horizontal scrollbar for Campaign.
  Scrolling preserves selection; keyboard navigation reveals the selected mission.
  The same component handles multiplayer mission browsing, save/load lists,
  shortcut bindings, and debriefing text, including the in-mission UI paths.
  All scrollbars reuse the original menu artwork.

- **Save mission time:** In-mission save info includes elapsed simulation time
  as minutes and seconds, including autosaves. Sherwood saves omit it; older
  catalog entries without a recorded timer remain readable.

- **Language-independent audio timing.** The required core-datadir
  `Data/AudioDurations.json` supplies English speech variants and sample
  durations to the client, replay preparation, and ranked verifier. A French-only
  installation needs no English audio pack. Localized recordings play to their
  natural end while simulation completes speech on canonical frames. Startup
  rejects a missing or invalid timing file; mission loading rejects missing
  required timing entries. The Rust `generate_audio_durations` example rebuilds
  the table and core inventory from English full-game and demo audio.
  See [generation instructions](../assets/core-datadir/README.md).

- **Mission details and previous plays:** Campaign Manager's Mission Details tab
  combines the original localized briefing, entry requirements, and a complete
  history across saved and archived campaigns. The briefing and play-history
  columns scroll independently under the pointer, with scroll-position indicators.
  Double-clicking a mission in the tree or gallery opens its details, including
  locked missions. Active unfinished missions are labeled In progress. Outcomes, dates,
  durations, and practice runs remain visible even without a recording.
  Watch Replay opens an available recording in a separate desktop viewer while
  preserving the current session. New terminal recordings are linked by exact
  attempt identity; older compatible local recordings are indexed in the
  background. See [campaign history](CAMPAIGN_HISTORY.md#campaign-manager-ui-and-offscreen-captures)
  for controls and browser limitations.

- **Campaign story navigation:** the main prerequisite route stays visible above
  stage-grouped story branches, optional missions, and ambushes. Training and
  campaign events are labeled separately; unused map placeholders are omitted
  unless they hold archived results. Mission Details explains actual money,
  gang, mission, expiry, and story restrictions, with keyboard and mouse scrolling.
  Achievement cards describe exact conditions and campaign-versus-mission scope.
  These presentation changes preserve campaign selection and award eligibility.

- **Campaign manager layout:** fixed 1024×768 presentation with twelve-card
  gallery pages, a navigable progress-tree viewport, wrapped mission titles,
  separate mission details, and three distinct views: Campaign for the current
  save, Hall of Deeds for permanent mission badges and best results across
  attempts, and Achievements for player-wide awards with current campaign
  progress beneath each. Whole-campaign awards require one qualifying campaign;
  loading an older save retains earned awards and archived best results.
  The scripted Sherwood ending is labeled Epilogue, placed after the finale in
  the tree and last in the gallery, without changing mission unlock rules.
  Both menu entry points and Sherwood share the layout. An opt-in offscreen GPU capture
  test produces PNGs for visual iteration without a game window; see
  [campaign history](CAMPAIGN_HISTORY.md#campaign-manager-ui-and-offscreen-captures).


- **Campaign manager from menus:** Campaign Manager on the main menu or
  Escape → Campaign Manager during any mission opens the existing progress
  tree / Hall of Deeds, including before reaching Sherwood. The main menu
  reads the selected player's latest resumable save, or a fresh campaign if
  there is no save. Navigation, history, and badge details use the same UI;
  mission launching is unavailable in this pause-side view. Escape returns to
  the originating menu without changing the active mission or campaign.


- **Leaderboard boards and custom gameplay settings.** The server publishes
  boards as plain configuration: edition, mission list, metrics and a
  simulation policy that is either one exact Standard/Original preset and
  difficulty or any validated configuration (including Legendary and Custom
  difficulty). Full-game field missions support individual runs. Verification
  checks the replay against the board's policy and independently reconstructs
  the canonical fresh campaign. Each run exposes its exact gameplay settings.

- **Scrollable mission debriefings.** Narrative, achievement conditions, and
  statistics scroll within the parchment using the scrollbar, mouse wheel,
  arrows, Page Up/Down, or Home/End. The footer controls stay visible.

- **Earlier browser replay downloads.** Admitted replay URLs prepare their exact
  mission selection once and start a bounded batch of required files before the
  normal mission loader, which reuses those requests. Five 16 Mbit/s pairs saved
  a median 172 ms with unchanged payload bytes; loopback showed no gain.
  See [measurements and validation](perf/replay-plan-earlier.md).

- **Recoverable save-store admission.** Damaged or inaccessible save indexes
  offer Retry and safe cancellation/exit without creating an empty replacement
  store. Save-picker errors are shown in yielding, scrollable acknowledgements;
  failed save operations also produce an in-game failure notice. Unpublished
  save drafts remain retryable but are not offered as loadable saves.

- **Shared scripted modal replay ownership.** Headless replay now shares scripted
  modal batch ownership with graphical sessions, supporting recorded aborted
  debriefings without discarding later-emitted batches. This does not add headless
  campaign/profile promotion or final load/restart presentation; those
  replay-required boundaries fail explicitly rather than report EOF.

- **Offline editor reconstruction.** `PIPELINE_OFFLINE=1` (or the AI CLI's
  `--offline`) permits reconstruction from validated provider caches without
  credentials or network calls. Missing or corrupt cache entries fail with
  actionable errors; completed cache publications are atomic.

- **Inspectable datadir conversion plans.** The converter writes
  `conversion-plan.json` with source dependencies and grouped output decisions.
  An incomplete checkpoint remains available if conversion fails; successful
  publication marks the plan complete. This diagnostic file is not part of the
  runtime payload or web content manifest.


- **Advanced combat gestures.** Swordfighting accepts nine optional
  single-stroke composite techniques in addition to the Original A-I gesture
  vocabulary. Composite recognition runs only after the original-game classifier
  returns `Attempt`, so enabling it cannot steal an already recognized legacy
  strike. Each technique expands into two ordinary authored sword commands,
  retaining their normal animation, interruption, targeting, energy, and
  protection behavior. An independent quality option quantizes template
  accuracy to deterministic 25/50/75/100 percent tiers and scales only cutting
  and concussion; protection RNG and geometry are unchanged. Optional guide
  and post-stroke coach overlays teach both the legacy and composite paths.
  Gameplay rules are authoritative mission state; presentation remains local.
  All four controls are independent under Gameplay, whose standalone screen is
  paginated so every row stays reachable at 640x480. Mouse and one-finger touch
  share the same recognizer, and replays/rollback/multiplayer carry the resolved
  typed technique and quality rather than platform-dependent pointer samples.
  Malformed, disabled, or mismatched payloads fail before simulation or
  quick-action recording. The incompatible native layouts advance to save
  **67**, replay **25**, and multiplayer protocol **34**.

- **Timed missions and runtime ambience.** Hackable JSON missions can author
  an active-play time limit and ordered Day/Night/Fog ambience cues. Timers
  pause with single-player/noninteractive simulation gates, stop once victory
  is achieved, and use the ordinary mission-loss flow at expiry. Ambience cues
  deterministically change AI perception, light-sector gameplay, filtered
  sound sources, lighting crossfades, sprite dictionaries, background art and
  minimaps; missing alternate art warns and falls back to Day. Gameplay and
  presentation/countdown controls are independently configurable. The hashed,
  serialized tick/cue/crossfade state is shared by saves, replay, rollback and
  multiplayer. `mods/timed-ambience-demo/` is a launchable example; authoring
  rules are documented in
  [Timed missions and runtime ambience](TIMED_MISSIONS_AND_AMBIENCE.md). This state
  advances the combined native save, replay, and network schemas to
  **65 / 23 / 32** respectively.

- **Cooperative pause side screens.** Options (including Graphics, Sounds,
  Shortcuts, and Gameplay), Save/Load,
  overwrite/delete prompts, and Quit confirmation now run as one-frame
  `ActiveUiTask` states owned by the mission loop instead of nested blocking
  async loops. Networking, HTTP control, replay bookkeeping, and frame
  stepping therefore keep reaching their normal outer-loop boundaries.
  Single-player retains the original paused-timeline behavior; a local menu
  in multiplayer captures only local input/presentation and does not stop the
  authoritative simulation. Window close propagates to mission exit, nested
  confirmations retain the picker underneath, stable save filenames protect
  selection across list mutations, and ordinary HTTP screenshots capture the
  presented topmost pause UI. Settings rows carry typed actions, enabled state,
  labels, and help text; large pages use bounded 12-row pagination, so the
  integrated Gameplay page exposes all 42 current settings without index or
  hit-box remapping. The native desktop data-folder chooser remains
  a synchronous OS dialog launched from the cooperative Options state.
- **Non-blocking mission-end leaderboards and verified-run consent.** The
  mission-end overlay opens verified score/time boards after wins, losses, and
  interrupted attempts, with a score and a time tab for the run's board.
  Board fetches, replay preparation, signing and upload are
  frame-polled tasks on native and browser builds; none waits inside
  rendering or pauses multiplayer. Submission is offered only for won
  missions, defaults to per-run consent, and has an explicit default-off
  **Always Submit Won Runs** preference. The independent **Mission-end
  Leaderboards** presentation setting defaults on. Both controls are
  available under Options → Leaderboards; neither can disable canonical
  replay/history capture or alter the ranked wire/storage format.

  Exactly one canonical compact replay enters an upload, and the signed
  submission names its exact digest and length. The client rejects
  non-canonical replay bytes and replays of another mission, and verifies the
  Ed25519 signature locally before uploading. Native and browser signing
  delegate to the shared durable leaderboard identity rather than creating
  another key store; the isolated browser signer exposes only username,
  submission, owner-status and deletion operations, never a generic signing
  primitive. Accepted uploads are tracked through authenticated owner-status
  polling until the server reports verification, rejection or failure.

- **Self-describing save metadata.** Every newly written manual, quick,
  rotating-autosave, continue, restart, and Sherwood save freezes its
  wall-clock timestamp, mission title, stable player-profile identity, and
  player name into both the payload header and lightweight slot index. The save
  picker shows mission/player provenance, honest relative age (including future
  times after clock correction), exact local time, and expanded campaign
  details. The per-profile **Detailed Save Metadata** option switches to a
  compact presentation without discarding stored metadata. Native saves
  require provenance, the multiplayer diagnostic-authority marker, and exact
  deterministic Spellforge package/journal state; incompatible older Rust
  schemas are rejected, and only the separate original-game importer may
  produce incomplete historical detail.

- **Mission badges and campaign achievements.** The accepted catalogue now has
  24 achievements: 10 mission badge types and 16 campaign achievement types,
  with **Clean Hands** and **Ghost** available at both scopes. The complete
  conditions and scope decisions are in [Achievement proposals](ACHIEVEMENT_PROPOSALS.md).
  [Gameplay validation](ACHIEVEMENT_VALIDATION.md) records live observations,
  original-data checks, and remaining proof routes. Mission metadata now filters
  the campaign badge catalogue as well as simulation results; the ransom feat
  requires the authored dispatch flag, and pursuit escape requires the party
  to remain on-map.
  The former all-enemies-stashed achievement is removed, and its stable ID 3
  is retired. New badges include **Ruthless**, **I'm off home**, beggar-info
  completion, banner purchase challenges, **Leave Everyone Standing**,
  **Not a Scratch**, and the generic-only optional-mission challenge.

  Campaign-only feats cover story completion/ransom, companions, permanent
  losses, camp workers who contribute in the field, civilian deaths, and
  one-time coordination, pursuit escape, capture, beer, wasp, and six-sling-
  knockout feats. Rich-civilian knockouts are counted by identity and remain
  credited after waking. The final hint payment is distinct from a later
  unrewarded donation. Deaths after initialization count for Kill a Civilian,
  including NPC and environmental/script damage; pre-existing corpses do not.
  Damage followed by healing still fails damage-free challenges.

  Tracking establishes a post-initialization NPC baseline, records authoritative
  effects, and includes its evidence in save/replay/rollback state. Clean Hands
  retains its configurable NPC-death rule; Ghost remains independent of kills.
  Campaign history practice restores progression deeds, so it cannot erase
  permanent losses or manufacture previous camp work.

  Results and metrics are frozen into the canonical attempt, and an exactly-once
  host attestation decides whether the badges may enter campaign and lifetime
  history. Custom, cheated, headless, and replay-playback runs remain auditable
  but do not award icons. A normal history replay is an eligible campaign
  practice attempt, so a player can return to one mission and earn a missed
  badge after the fact. Campaign badges, detailed debrief conditions, exact
  selected-character sword/bow XP, the speedrun clock, and the optional
  top-left achievement trackers have separate settings. Fresh profiles show
  campaign badges and debrief details but leave the HUD trackers and detailed
  XP off; settings documents that predate these presentation fields leave all
  of those presentations off without discarding calculated history. The authoritative rules live in
  `crates/robin_engine/src/achievement.rs`; presentation lives in
  `crates/robin_rs/src/achievement_hud.rs`.

  Mission-only badges do not create duplicate campaign awards. **Clean Hands**
  and **Ghost** require the badge on every successful canonical mission in one
  completed campaign path; one-time campaign feats need one eligible success.
  The campaign catalogue is paged, while the classic map shows compact totals.
  The expanded deterministic state uses native save 74, replay 32, network 41,
  campaign history 3, and profile history 4. Older native schemas are rejected;
  original-game imports retain incomplete evidence. The required path is
  frozen by campaign run ID and terminal sequence when the Original campaign
  completion boundary is crossed. Lost, failed, and interrupted attempts stay
  in full history but never expand that won-mission set. A later practice
  replay may fill evidence for a mission already in the envelope, but cannot
  add a mission or combine evidence from another campaign. Completed envelopes
  live in the profile's lifetime archive and survive campaign reset.
  Incomplete original-game imports are shown as unverifiable until real eligible
  evidence exists; import never fabricates an award.

  Deterministic tracker fields and the NPC-death gameplay rule are part of the
  native state contract. This feature therefore advances native saves to v58,
  replays to v18, and multiplayer to protocol v25; obsolete Rust layouts fail
  closed instead of being decoded as plausible achievement evidence.

- **Rotating autosaves.** Single-player missions create interruption-safe
  recovery points at mission boundaries, when the native window or browser page
  is backgrounded, and every five minutes of active gameplay. One ordered
  writer commits an immutable full save payload before publishing a separate
  manifest, retains exactly the newest three generations, drains accepted work
  during clean shutdown, and removes unpublished crash leftovers on recovery.
  Browser records use the same portable JSON value model inside compressed,
  SHA-256-checked storage envelopes; lifecycle events wake the game before
  hidden-page timer throttling and urgent snapshots publish synchronously.
  Autosaves appear only in Load lists and cannot be manually overwritten or
  deleted. The independent Gameplay option is enabled by default. Multiplayer,
  replay playback, and headless automation never autosave.

- **Legendary and custom difficulty.** Player profiles may select the three
  retail presets, a fixed Legendary preset, or a validated Custom ruleset.
  The resolved integer/boolean rules are authoritative simulation state and
  travel through profiles, saves, replay snapshots, and multiplayer welcome
  messages. Legendary continues the retail combat/health/reaction/capacity
  progression and additionally gives hostile soldiers 135% view distance,
  125% view-cone width, and 150% noise sensitivity. Those perception rules
  are independently editable in Custom; Easy, Medium, and Hard remain at
  their original 100% perception. Original-RNG parity maps Legendary/Custom
  back to their explicit retail compatibility preset. Exact Standard/Original boards
  retain immutable Easy/Medium/Hard identities; open boards also accept Legendary
  and Custom settings. Story mode is
  intentionally absent.
  - TODO: what can we do to make legendary harder without being boring?
    - just increasing health is boring
    - increase vision distance or vision cone angle
    - increase hearing sensitivity (or PC make more noise)
    - decrease unconcious time
    - longer distance for archers / crossbowmen (NPCs) and opposite (PCs) - although making items less useful is boring as well
    - smarter enemy AI - e.g. archers should try to avoid and escape hand to hand combat more - normally once you have an archer in normal combat you've basically won instantly. what else?
    - MORE enemies (can we programmatically just increase NPCs in missions?, just add more randomly somehow)
      - make patrols larger (commander + 4 soldiers -> 6 soldiers)
      - add more enemies in same spread (e.g. 5 archers on one walkable surface (wall) -> 7 archers spread out on same surface?)
      - needs tweaking / candidate review
    - no saving / permadeath / no clovers for reviving (?)
    - enemies look around more

- **Untie tied NPCs.** A PC with the Tie skill can click any living tied NPC
  to release them, using the rope cursor and the authored tying animation in
  reverse. Search remains the first contextual action while the NPC carries
  loot. Untying preserves unconsciousness and concussion so normal recovery
  remains authoritative, works in Shift queues and replay/multiplayer command
  streams, and can be disabled under Gameplay to restore shipped behavior.

- **Bounded widescreen and native-resolution presentation.** The three
  original logical scale presets remain 640x480, 800x600, and 1024x768, while
  an enabled-by-default Graphics option adapts the logical canvas to window
  aspect changes. High reaches 1280x720 at 16:9; wider displays are
  letterboxed, portrait/narrow displays retain 4:3, and every preset is capped
  inside the 1280x768 gameplay envelope. The physical swapchain remains
  independent, including HiDPI browser canvases and fullscreen output. Resize
  propagation updates pointer mapping, camera bounds, minimap, HUD, and modal
  return paths without changing the three original camera zoom levels. Wider
  portrait bars expose additional slots, fixed-width border art fills the
  complete width, and Android uses a cutout-safe immersive content area.
  Disabling Adaptive Widescreen restores fixed 4:3 parity presentation.

- **Live Sherwood production forecast.** The Sherwood report now includes a
  compact, toggleable item-production panel built from current map stock and
  live production-zone membership. Once a mission is selected it shows exact
  output for that mission's authored duration; before selection it reports an
  exact one-hour rate rather than guessing a mission. Each item line exposes
  current stock, five-per-production-point capacity, overflow, worker and
  specialist inputs, authored speed, and the original game's explicit lack of
  raw-material consumption. Forecasting and campaign production call the same
  pure calculation, with boundary tests guarding truncation and saturation.

- **Authoritative Sherwood item trading.** Hosts can sell any stored production
  item for a fixed, documented ransom value through an independently toggleable
  Sherwood panel. The built-in **T** action and the touch/mouse-accessible pause
  menu row share one fail-closed modal request; both require Sherwood, the host
  seat, and the enabled gameplay rule. Explicit **Sell 1** and **Sell 5**
  actions require a second matching activation, then exact stock removal and
  ransom mutation occur only in the deterministic command frame. Replays,
  rollback, multiplayer, saves, and Original-parity policy carry or reject the
  same typed command and configuration.
  Controls, prices, production rationale, and integrity rules are documented
  in [Sherwood item trading](SHERWOOD_TRADING.md).

- **Data-driven mission allegiances.** Hackable JSON missions may assign a
  numeric `allegiance` to each soldier and rescue PC. IDs `0` and `1` preserve
  the legacy Royalist and Lacklandist camps; any `u16` ID is accepted, and
  distinct valid allegiances are mutually hostile unless an optional
  `diplomacy` block declares a symmetric `allied`, `neutral`, or `hostile`
  relationship. The block can also declare a multi-allegiance
  `player_coalition`; coalition members are always allied. Runtime target detection,
  combat, minimap classification, cursor actions, difficulty modifiers, and
  NPC-enemy availability use relationship queries instead of assuming one
  opposing camp. Legacy RHM actors without the optional field still derive
  their allegiance from the original hostile/attitude profile flags.
  Hackable descriptors also accept `spawn_player`, `soldiers`, and `pcs`;
  soldier `profile` values may use readable CPF-filename identifiers such as
  `guard_a01` (legacy numeric indices remain accepted);
  AI-controlled heroes with `decision_policy: "enemy_ai"` run through the normal enemy
  perception, pursuit, target-reacquisition, and battle-decision lifecycle
  instead of receiving one-off mission-start pairings. Their required readable
  `ai_profile` (for example `soldier_b04`) selects the behavior personality;
  the PC profile still supplies the hero's weapons, skill, endurance, sprites,
  damage, tiredness, and reactive defence.
  Ten launchable test arenas share the `mods/multi-team-demos` mod:
  three-way, ten-way, every soldier/PC profile in unique-allegiance circles,
  autonomous Robin versus Little John, four armies of twelve soldiers, and a
  four-army matchup where each faction uses a distinct soldier grade, and an
  all-hero ten-way circle free-for-all, four archer companies in crossfire,
  twenty Robins against twenty Black Knights, and four champions with mixed
  soldier retinues.
  Relationships can change deterministically during play through
  `GetDiplomacyRelationship` / `SetDiplomacyRelationship`, the
  `DIPLOMACY <first> <second> <allied|neutral|hostile>` console command, or
  serialized player commands. A change immediately reconciles perception,
  AI targets, swordfights, projectile protection, cursors, and minimap colours;
  neutral factions use amber. Options → Gameplay can disable authored
  diplomacy or all NPC-vs-NPC faction wars, and Options → Graphics can disable
  relationship-aware colours. Per-mission statistics retain deterministic
  per-allegiance soldier encounters, survivors, deaths, and player-caused
  deaths alongside the legacy aggregate rows. The three-way test arena is an
  editable allied/neutral/hostile example.

- **Orthogonal human-actor roles and control.** `Pc`, `Soldier`, and
  `Civilian` now describe body/profile archetypes only. Runtime and hackable
  level data independently describe decision ownership (`player_directed`,
  `enemy_ai`, `friendly_ai`, or `scripted`), the player command surface
  (`hero_actions`, `tactical_orders`, or `none`), mission bookkeeping role,
  combat stance, and allegiance. This permits AI heroes and tactically
  commandable soldiers or villains without pretending that body type or camp
  determines control. Legacy RHM Royalist troop commandability and the old
  custom-level `autonomous`/`aggressive_combat` fields are translated once at
  load time. Existing replay command names and structured snapshot keys remain
  readable for compatibility. Rescue heroes explicitly transition from
  `rescue_target`/no commands to `player_party`/hero actions when the original
  `CharacterAvailable(true)` sequence fires.

- **Mission-selective shipping data.** Converted shipping datadirs now contain
  a compact boot manifest plus independently compressed mission cores and
  shared per-character RHS sprite payloads. Native, Android, and browser builds
  load only the files referenced by the selected mission; decoded files remain
  cached when missions share characters or when the player returns to a
  mission. Per-RHS grouping deliberately preserves the strong within-character
  zstd matches measured in `docs/COMPRESSION.md`.

- **Browser-native shipping audio.** Web shipping conversion transcodes voice,
  effects, and music to deterministic Opus dependencies while retaining exact
  source-duration metadata for simulation timing. The browser fetches and
  decodes only boot audio plus the selected mission's closure, reports that
  work on the loading screen, and keeps decoded PCM in Web Audio buffers rather
  than wasm linear memory. Native and Android artifacts retain source audio.

- **Planned quick-action queue.** Holding the rebindable `Plan Quick Actions`
  control (Shift by default) switches the portrait
  action buttons, cursor, and projectile preview to a separate planning state:
  selecting Bow or an item does not equip it, stop the hero, or otherwise
  mutate the live PC. World clicks use the QA macro system and execute in
  order, with no three-action queue limit and without consuming the Original
  three explicit macro slots. The first action starts as soon as the actor's
  existing work finishes; its slot is consumed immediately, while pending work
  remains visible in an independently animated strip. Plan-double-click upgrades
  the newest pending movement to a run. Planned actions may be selected even
  when their live ammo is empty, and releasing the control or plan-right-clicking
  clears the planned action without touching the live PC. Bow arcs are previewed from the last
  queued or live movement destination, so targets can be planned from the
  position the hero will actually occupy. Shield and Big Shield retain their
  exact protectee-then-danger-point interaction. When the separate tactical-unit
  control extension is enabled, directly controlled units can queue formation
  movement and combat, with group-portrait queue feedback. Touch-capable builds
  expose a sticky plan/cancel HUD button. The per-profile
  Gameplay setting disables all live planning UI/input and defaults on;
  Original-parity replay forces it off.

- **Self-updating native packages.** Installed Windows and Linux Velopack
  builds check the public GitHub Releases feed in the background. Stable
  installs consider stable releases only, while prerelease installs also
  follow dated `nightly-YYYY-MM-DD` prereleases. Downloads do not interrupt
  play: completed updates are applied silently after a normal game exit, and
  a previously downloaded pending update is applied before the next startup.
  Headless games, standalone archives, and developer builds do
  not attempt to update themselves.

- **Startup datadir selector.** When `ROBINHOOD_DATA_DIR` is unset, the
  game resolves the data folder itself: a previously confirmed choice
  (remembered in `datadir.txt` next to the saves) is used silently;
  otherwise it auto-detects an installation — working directory,
  executable directory, then the usual install locations of the original
  CD (`Program Files\Wanadoo Edition\<localized title>`, per the Wise
  installer script), GOG (GOG Games, GOG.com, Galaxy), and Steam,
  plus Wine/Heroic/Lutris prefixes on Linux — validated via
  `Data/robinhood.bks` (case-insensitive; `Data/datadir.bin` shipping
  bundles also count). A native dialog always confirms the result: OK
  accepts the found installation, Cancel opens the OS folder picker. The
  dialog recommends a GOG purchase. The remembered folder can be changed
  later via Options → "Game Data Folder" (applies on next launch).
  Headless runs use the auto-detected folder without a dialog and keep
  the descriptive terminal error otherwise.

- **Core overlay datadir.** `assets/core-datadir/` is registered as an
  always-on overlay ahead of the `mods/` overlays. It currently restores
  the game's native bitmap fonts (~280 KB) plus the font `manager.cfg`,
  fixing the Steam release — whose depot ships only the international
  TrueType (SimSun) font set and therefore renders every menu in a
  Windows system font in the original build too. It also carries 19 engine UI
  PNGs, including the allied controls and villain portraits. Packaged native
  targets validate the strictly sorted 32-file size/SHA-256 inventory in
  `core-overlay-manifest.json`; desktop startup validates the exact physical
  tree before mounting it, while Android packages it as a retail-content-free
  asset root and validates every entry before UI startup. Browser builds
  explicitly preload only their build-reachable font/UI subset. See
  `assets/core-datadir/README.md`.

- **Runtime language switching.** Main-menu Options discovers and validates
  every installed retail locale and offers an application-global `Automatic`
  or explicit language choice without restarting the game. Active-mission and
  pause-menu Options deliberately do not expose the selector; the selected
  language applies when the next session is built. Applying a language
  atomically replaces loose-datadir or shipping-bundle lookup, rebuilds eager
  menu presentation caches, and returns to Options. Text never silently falls
  through to a different language; only missing recorded speech and
  cinematics may use an installed English pack. Shipping datadir format v15
  preserves complete per-locale overlays for native, Android, and browser
  packages; older manifests fail loudly and must be regenerated. Generated
  character names and persisted save labels remain frozen, multiplayer mission
  text and playback are client-local, and logical speech timing comes from
  the required English core audio timing table, independent of installed voice
  packs and the client's active presentation language.

- **Hackable JSON levels.** Every subdirectory of `mods/` is registered as an
  overlay datadir at startup, and any overlay may ship an editable
  `Data/Levels/<mission>.level.json` geometry descriptor (title, spawn point,
  walkable polygon, architectural volumes) that expands into normal level
  structs at load time — no legacy RHP/RHM/terrain encoding involved.
  Backgrounds and minimaps can be plain PNGs, optionally paired with a 16-bit
  `<map>.occlusion-depth.png` for continuous sprite occlusion. Discovered
  levels get a main-menu entry (descriptor `title`) and can be launched
  directly with `--mission <name>`; they run as unscripted sandboxes.

- **Direct custom-mission launch.** `--custom-mission <zip>` mounts a vanilla
  mod archive for the lifetime of a direct `--mission <name>` launch. Pair it
  with `--proto <map>` when the mission and proto-level basenames differ.

- **Optional tactical-unit control.** A persistent `Control Tactical Units`
  game option enables high-level control of actors whose level data exposes
  `command_interface: "tactical_orders"`, independently of allegiance. Click
  or drag a selection box to create a temporary portrait beside the heroes;
  its illustrated pin button preserves an individual or group portrait.
  Named soldier profiles can supply dedicated 112x50 visage art; Guy of
  Guisbourne, Longchamp, Prince John, Scathlock, and the Sheriff use portraits
  cropped from the Original's dialogue resources, while ordinary and mixed
  groups retain the helmet portrait.
  Level JSON patches can preserve compiled-script actor indices while
  overriding beam-me, rescue-PC, and tied-prisoner visuals. A per-mission
  `.descriptors.patch.json` can override popup, short-briefing, and dialogue strings
  while retaining the base mission's descriptor pictures and timing.
  Mod-added character profiles keep their visible name and NPC exclamation
  bank separate from the internal RHS profile key, so promoted villains retain
  their own voices. NPC sprite sets used as PCs also fall back to compatible
  authored attack rows for target interactions they do not natively animate.
  Promoted NPCs can import their retail soldier combat statistics while
  retaining a playable character template's action layout. Mods can remove
  inherited contextual actions unsupported by the NPC sprite.
  Allied portraits
  expose cycling hold/defensive/aggressive stances, two-point patrol targeting,
  and type-aware line, box, staggered, and flank formations. Selecting soldiers
  visualizes both player-issued patrols and authored mission patrol paths as an
  animated dotted world-space chain with waypoint and next-destination markers.
  Line formation
  places officers at the command center, knights in close escort, shield and
  melee troops on the fighting edge, and ranged troops in protected rear or
  central positions. When heroes move with the selection, soldiers deploy
  behind them instead of overlapping their formation. Long moves automatically
  narrow to a two-wide marching column before deploying at the destination.
  Double-click runs allied soldiers even when a drag selection also contains a
  hero.
  Controlled soldiers can enter swordfights, execute the normal drawn strike
  gestures, and parry once while releasing the allied selection with
  right-click. Combat autonomy follows stance: Hold
  permits only explicit gestures, smalltalk, and reactive parries; Defensive
  returns attacks without AI pursuit; Aggressive retains full combat AI.
  Explicit gestures supersede combat work the soldier AI had already queued. Hover
  tooltips name every action and its current state and appear quickly across
  each button's full cell. Soldiers receive deterministic names from the
  localized peasant-name pool. Selection uses the heroes' persistent ground
  ring and fading green outline. The portrait bar
  computes its capacity from the actual screen width (six portraits fit at
  800 px) and uses the original Sherwood left/right arrow resources with
  wraparound paging when the combined hero and allied portraits overflow.

- **Optional Hard reaction-time fix.** Options → Gameplay includes a
  persistent `Fix Hard Reaction Times` toggle. When enabled, Lacklandist NPCs
  on Hard difficulty use the intended `HARD_REACTIONTIME_MODIFICATOR` instead
  of the Easy modifier selected by the original game's copy-paste bug. It is
  on by default; the original-parity replay tool disables it explicitly.
  Changes made during a mission take effect immediately through the
  deterministic command stream.

- **PC-centred fog of war and re-hide intelligence.** Options → Gameplay
  includes an off-by-default `Fog of War` toggle. The world and minimap now use
  unexplored, explored/dimmed, and currently visible polygon regions derived
  from active playable-PC vision plus opaque line of sight. Hostile actors disappear outside
  current sight after a short deterministic hysteresis and leave a stationary
  minimap marker that fades over ten seconds. Listen grants a temporary
  reveal, allied NPCs do not create reveal circles, and script-authored minimap highlights
  pierce fog. The authoritative current region is the exact union of the PCs'
  Original-compatible 3D shadow polygons, and explored terrain is their
  cumulative polygon union. World and minimap presentation rasterize those
  regions at up to one texture pixel per map pixel; GPU linear sampling
  antialiases that pixel-scale result. The live boundary is evaluated from each PC's exact position using
  1.2× the mission's standard vision range, producing smooth, progressive
  motion without cell transitions. Both polygon sets are hashed and serialized
  in saves, rollback, replays, and multiplayer snapshots; the alpha texture is
  only a cached presentation derivative. Visibility is refreshed each simulation
  frame through the same 3D obstacle projection used by the view overlay.
  Actors query the current ground polygon directly, keeping disclosure exactly
  aligned with the clear ground beneath them. World presentation constructs the
  authored closed 3D mesh for every sight obstacle (top triangles and side
  triangles) over a `z=0` map plane. Player shadows are projected onto each
  triangle's own plane, while pairwise affine isometric-depth clipping retains
  only the camera-nearest surface at every projected position. The resulting
  visible and explored unions therefore expose real walls, roofs, ramps, and
  elevated ground without facade expansion, bounded-top guesses, painter-order
  dependence, or whole-building projection-hull leaks. Unchanged observers
  reuse a geometry-signature cache; eye movement or any static/dynamic obstacle
  change invalidates it exactly. The cache is presentation-only and is omitted
  from saves, rollback snapshots, replay payloads, and deterministic hashes.
  Large patch animations
  defer visibility to the final per-pixel fog composite rather than being
  rejected by a single hidden anchor point, so replacement artwork is clipped
  by the exact fog boundary without clearing terrain behind it.
  Camera
  movement alone reuses the cached GPU texture. This state is deliberately
  separate from the Original's
  permanent `blipped` identity-discovery flag, so `UNBLIP` behavior is
  unchanged. Disabling the setting takes the original rendering/input path;
  original-parity tooling forces it off. Full-map exports also default it off
  and expose `--fog-of-war` when a fogged capture is desired.

  This adds serialized deterministic state and a replayed toggle command.
  The complete current graph, including cold custom-mission identity, uses
  native save schema 72, replay schema 30, and multiplayer protocol 39;
  obsolete incompatible Rust layouts are rejected.

- **Original-compatible 3D view overlays.** View cones now project complete
  opaque obstacle volumes independently onto the ground and every visible
  platform plane. As in the Original, feet and head shadows are evaluated
  separately and an area remains visible when either endpoint can be seen.
  This replaces the earlier two-vertex ground-wedge approximation, including
  its incorrect elevated and through-wall results. The projection core is
  renderer-independent and accepts arbitrary polygonal domains and target
  surfaces, so precise fog masks can reuse the same geometry instead of
  maintaining a second obstacle model.

- **Fog/night-tint all sprites option.** Options → Graphics now includes a
  `Fog/Night All Sprites` toggle. On fog and night missions it applies the
  generated ambiance sprite variant to Day-based world sprites, including
  bonuses, scrolls, animals, and mobile child sprites that the original leaves
  in the Day palette. Animation-backed FX and targets already load
  ambiance-specific pixels and are excluded to prevent double tinting. New
  profiles enable it by default; the toggle can restore original behavior.
  `render_mission_map` exposes explicit `--fog-tint-all-sprites` and
  `--no-fog-tint-all-sprites` overrides for reproducible A/B captures.

- **Quick-action recording cursor pulse preference.** The classic three-slot
  recorder uses the original opaque RGB565 shadow pulse, from black to pale
  yellow-green. Its 20-step triangle runs on a cursor-local clock (40 ms per
  step), independently of simulation, pause, rewind, and replay time. Options →
  Graphics → `Quick-Action Cursor Pulse` can disable it. Bow-target shadow
  colors apply normally outside recording. The effect's animation state is
  never saved or recorded.

- **Mission-start full-map renderer**
  (`crates/robin_rs/examples/render_mission_map.rs`). Loads a mission through
  the regular engine and GPU renderer, captures the complete map after the
  requested number of normal simulation frames (`--frame`, default zero), and
  writes a HUD-free PNG. `--reveal-all` (alias `--unblip-all`) switches every
  NPC from its blip silhouette to its normal character profile before capture;
  `--headless` keeps the screenshot renderer's GPU-backed window hidden.
  Complete exports bypass gameplay fog unless `--fog-of-war` is requested.
  `scripts/render_all_mission_maps.sh [OUTPUT_DIR] [FRAME] [DATA_DIR]` renders
  every shipped mission using the full-game profile mapping and human-readable
  mission titles as filenames.

- **Local script-RPC HTTP server** (`crates/robin_rs/src/http_server.rs`).
  Loopback HTTP access to script natives, engine inspection, screenshots, player
  commands, and replay control for external debug tools, test harnesses, and AI
  drivers. See [HTTP automation server](JSON_SERVER.md) for setup, endpoints,
  and examples. The HTTP transport is desktop-only; Android disables it, while
  browser builds expose the same queue through the `rh_rpc` JavaScript bridge.
  Replay recording uses a mission-generation-scoped, 64 MiB segmented spool
  that publishes only complete flush boundaries and fails closed on overflow
  or durable-writer errors. Export has single-flight backpressure and produces
  the canonical compact bitcode replay through `robin_replay_format`, with no
  alternate JSONL export or ranked format.

- **Upscaling and presentation effects**. Options -> Graphics -> Scaling now
  ships a portable multi-pass wgpu runner. It includes Nearest, Linear,
  Pixel Art/Sharp-Bilinear, Bicubic, Lanczos, CUT3, a published-corner-rule-
  derived **ScaleNX** path with artifact removal, and clearly labelled clean-room
  **HQx-style**, **xBRZ-style**, **Super-xBR-style**, and **Anime line A/B/C
  (v4 layout)** profiles. The Anime profiles follow Anime4K v4's documented
  restore/soft-restore/denoise pass ordering, but intentionally do not claim
  to reproduce Anime4K's trained kernels.
  - CRT is an independent, disableable `TextureEffect`: None,
    **CRT Guest-class**, or **CRT Royale-class**. Both implementations are
    original portable WGSL inspired by those shaders' documented controls;
    no GPL shader code is embedded.
  - Strength, edge threshold, artifact removal, scanlines, phosphor mask,
    bloom, curvature, and presentation-rate temporal flicker are persisted
    per profile. Temporal state advances only after a frame is submitted for
    presentation, independently of deterministic simulation ticks.
  - The world/video layer is scaled and effected first. Menus, HUD, cursors,
    and modal overlays are then alpha-composited with sharp-bilinear sampling
    so display effects do not blur text.
  - Standard native builds can choose bundled `.slangp` presets or import and
    compile an external preset from the Graphics screen. Preset
    parse/compile/frame errors are reported; a failed preset never silently
    falls back. Browser builds hide this unavailable choice while retaining
    all portable WGSL profiles on WebGPU and WebGL2.
  - Algorithm provenance, exactness, licensing, platform support, and shader
    restrictions are documented in [Upscalers](UPSCALERS.md).

- **Deterministic replay and rollback checking**. Sessions can be recorded to
  JSONL locally and exported as binary `.rhrec` files: `RHREC` plus byte `1`,
  a 12-byte lowercase hexadecimal build identity, then Zstd-compressed bitcode.
  Export, ranked upload/storage/download, native loading, and browser worker/RPC
  playback preserve binary bytes. The media type is `application/x-robin-rhrec`.
  This is a clean break: old `rhrec-...` text artifacts are rejected. Only share
  URLs wrap binary bytes in Base64url (`?replay=rhrec1-...`). Native `--replay`
  takes a file/archive path; `GET /get-replay` returns binary, and native
  `POST /load-replay?paused=false` accepts the binary body and replay media type.
  Ranked ingestion accepts binary artifacts only and enforces independent input,
  compressed, decompressed, Zstd-window, frame, metadata, campaign, and work limits. The rollback checker periodically
  replays recent frames from a snapshot and compares the reconstructed engine
  state against the live state to catch nondeterminism. Explicit playback
  requests are decoded before Engine construction and fail fatally if their
  required header cannot be read; playback never substitutes a multiplayer or
  default RNG seed. Gameplay randomness uses one serialized Engine-owned
  `fastrand` stream instead of Original's process-global C RNG; parity is
  reviewed at ranges and call-site order rather than bit-identical rolls. The
  reviewed inventory and host-only exceptions are enforced in code; typed
  serialized-stream labels and a separately typed seed-derived authoritative
  peasant-name generator, plus a structural source test, reject unreviewed
  gameplay RNG additions.
  Each mission recording is a directory of append-only JSONL chunks, indexed
  by `mission.json`. Every save capture (including background autosaves and
  Restart) writes and flushes a marker before the save can be published. Saves
  reference the directory, chunk, and marker, binding the reference to the
  complete captured payload. Markers occupy host-only records: queued gameplay
  commands execute afterward, without a fabricated simulation tick.
  Every load starts a new chunk with both a chronological predecessor and the
  restored save reference. Mission-wide ordinals preserve all abandoned gameplay,
  saves, and reloads, including when the application is restarted. Export and
  leaderboard submission use one self-contained artifact assembled from the
  complete chronology; playback needs no original save files. An individual
  chunk path replays history through that chunk, so earlier attempts remain
  watchable after subsequent loads. `--record <directory>` creates a new mission
  directory; `--replay <directory>` plays its complete history. Existing
  standalone JSONL and compact replay files remain supported. Browser and native
  playback consume recorded terminal updates without opening live debriefing or
  leaderboard flows, so abandoned wins/losses can be followed by another restore.
  Timeline scrubbing addresses recording ordinals, so repeated simulation
  frames across saves and reloads remain individually reachable. Backward seeks
  reconstruct from the mission start; forward seeks execute every intervening
  record. Recorded simulation gates remain authoritative during modal playback.
  Native and wasm use fixed-width random index draws for campaign names and
  simulation shuffles, preserving the native stream across both platforms.
  Missing or invalid referenced history is reported explicitly; it cannot
  become leaderboard evidence. Fully verified marker restores qualify for the normal leaderboard:
  verification executes abandoned gameplay too, and restores only states derived
  from verified save markers. The mission directory retains the original signed
  admission for resumes across application restarts. Release rules permit these
  complete histories; embedded foreign snapshots remain ineligible. Browser
  chunks persist in fixed-size IndexedDB blocks, with a bounded synchronous
  write-ahead journal protecting urgent save markers. Storage/quota failures
  are explicit; recovery handles a page closing before journal retirement.

- **Original-game parity traces**
  (`crates/robin_parity/src/original_parity_replay.rs`). A diagnostic runner
  streams the neutral JSONL trace emitted by an instrumented original game,
  applies its resolved player commands on the recorded frames, and compares
  typed entity state using exact floating-point bits. Unsupported legacy
  command values and malformed/non-contiguous traces fail loudly; the first
  divergent frame is reported field-by-field.

- **Shared story confirmations**. After accepting a story scroll or dialogue,
  players see the names still waiting to confirm. The host broadcasts
  acknowledgement progress while retaining authority over dismissal. Story
  openings are announced by the host, so delayed clients cannot lose them
  when prediction rollback silently reconstructs past simulation frames.

- **Basic multiplayer**. Native host/client networking over iroh
  (peer-to-peer QUIC with relay fallback; peers addressed by endpoint id, no
  port forwarding), seat IDs, input delay, rollback for late inputs, mission
  seed sync, state-hash desync detection, mid-mission state snapshots for
  joiners, and client reconnect are implemented. Matchmaking is fully
  serverless: the multiplayer menu joins a well-known iroh-gossip topic
  bootstrapped through the BitTorrent Mainline DHT, so games are discovered
  with no broker, master server, or configuration. Matchmaking `/1` host
  announcements are signed by the persistent game identity, expire after a
  short validity window, bind the advertised endpoint to the signer, and use
  per-host issuance watermarks so captured older lobby state cannot roll a
  listing backward. The current design is predictive rollback netcode rather
  than strict "wait for every peer before ticking" lockstep. Inputs older than
  the retained correction horizon now force a complete transport reconnect
  and fresh authoritative snapshot instead of being applied at the wrong
  frame.

- **Unified mission timeline and non-blocking multiplayer UI**. Rewind,
  multiplayer correction, and rollback verification share one mission-owned
  command journal with dense recent checkpoints and whole-mission rewind
  checkpoints every 10 seconds. Older checkpoints use bitcode + zstd with
  reusable codec buffers; interactive rewind caches at most 25 live states.
  Snapshot/load adoption seeds an explicit checkpoint at its exact
  frame, including between normal sparse boundaries. Blocking gameplay modal
  traffic uses client proposals and host-only decisions; remote peers cannot
  choose the host's restart or load outcome. Campaign map/description and
  launch confirmations, cross-mission QuickLoad confirmation, pseudo-mission
  debrief, lost-Sherwood, and terminal mission-state/debrief/load flows retain
  state across outer frames so networking and replay services keep draining
  while simulation is paused. A player's pause menu is a local overlay in
  multiplayer and does not stop the shared simulation. HTTP timeline stepping
  accepts typed modal outcomes, defaults to automation-friendly auto-dismiss,
  validates each result against its modal kind, and reports or blocks unresolved
  UI explicitly. Ordinary keyboard/HTTP timeline movement is disabled during
  multiplayer; explicit host automation must opt into synchronized stepping,
  after which every peer reconnects from the resulting authoritative snapshot.
  Local keyboard and HTTP pause changes are rejected in multiplayer so one peer
  cannot stop only its own timeline.

- **Fast verified-replay seeking**. The validator produces an optional zstd-19
  checkpoint sidecar without modifying the signed replay. Browser playback
  downloads and validates it in the isolated admission worker, then recompresses
  individual checkpoints into the same bitcode + zstd memory cache used by local
  seeks. Checkpoints are keyed by replay ordinal every 250 records, so save/load
  branches remain distinct. The sidecar includes saved-state dependencies and a
  sparse modal-effect journal; missing or incompatible artifacts fall back to
  normal replay. The sidecar is served only for publicly accessible runs.

- **Host-authoritative multiplayer session transitions.** Load, Restart,
  QuickLoad, and Sherwood campaign launch use a prepare/ready/commit barrier.
  The host encodes authoritative save or campaign state once, every connected
  peer validates and retains those identical bytes, and only then do all
  participants tear down the old mission transport and enter the next ready
  barrier. Load/Restart controls remain disabled for clients and throughout a
  transition. Campaign hosts write resumable saves; other online Save and
  QuickSave requests create tagged local diagnostic captures: connected load pickers hide them and the central transition path
  rejects them even if UI filtering is bypassed. Sherwood campaign UI remains
  non-pausing, but only host-authored campaign commands can mutate simulation
  state. Client modal choices remain visible host proposals; only the host can
  publish the decision that closes a shared modal. Replacement missions
  consume a one-shot continuation containing the same session id,
  authenticated owner/seat roster, expected player count, and pinned relay
  route. Durable browser identities and process-held native client identities
  therefore reclaim the same seats without nickname authority or a stale-relay
  reconnect race. The native server rejects peer-authored deterministic
  settings, campaign mutations, modal decisions, and seat-lifecycle commands
  before broadcast; the engine repeats that check at deterministic command
  admission so replay/rollback cannot bypass transport authority.

- **Authenticated browser multiplayer**. A native host can publish a
  30-minute, fragment-only `rhmp3` invitation for
  `https://robinhood.phiresky.xyz/`. Browser peers use iroh's
  relay-over-WebSocket transport with the protocol-34 game wire,
  prove a durable non-extractable identity through an isolated typed signer,
  and reclaim only their parked seat generation. Demo and Full joins fail
  before boot unless the ticket-selected engine artifact, exact native
  Data/locale closure, and every browser package byte agree. Reconnect adopts
  an authoritative replacement snapshot even when it predates the abandoned
  prediction future, then clears future inputs/hashes/history. Only the host
  records the canonical server-ordered replay. Relay observability is stated
  in the invitation UI, and browser-link publication is a default-on persisted
  privacy setting that can be disabled without affecting native iroh play.

- **Deterministic Spellforge Lua missions**. A versioned package contract
  supports upstream same-name Lua replacement plus explicit SCB augmentation.
  The engine callback driver dispatches Initialize/PostInitialize, one-second
  timers, three-second victory checks, Finalize, messages, keys, and all actor,
  target, scroll, zone, and waypoint events. Native calls suspend through the
  same synchronous engine operations as SCB. Exact package bytes/hash and a
  nested event/native tape are authoritative state. The tape is an append-only
  persistent journal: per-frame rollback clones share their immutable history,
  state hashes consume a rolling SHA-256 chain instead of walking every prior
  event, and serialization flattens the journal to a non-recursive wire list.
  Event, native-call, argument, nested-transcript, retained-byte, and combined
  package/tape snapshot ceilings are part of the executable ABI; reaching one
  aborts the mission with a typed resource-limit diagnostic. Arbitrary Lua heap
  and callback closures reconstruct after save/load, rewind, rollback, replay,
  headless execution, multiplayer snapshots, and reconnect without reapplying
  engine side effects. Snapshot admission recomputes the package digest and all
  journal accounting before state hashing or level attachment. Missing
  packages, hash/version/ABI mismatches, script failures, and tape divergence
  are fatal rather than SCB fallbacks. Profiles
  can disable Spellforge missions in Gameplay settings; required replacement
  missions then stop with an explicit launch error. Native and browser builds
  execute gameplay through the same safe-Rust Lua 5.1.1 VM and conformance
  suite; browser launchers can hand archive bytes directly to the in-memory
  package loader, with no JavaScript evaluation or Emscripten side runtime.
  The author contract, limits, checker workflow, corpus gate, and unresolved
  product-policy seams are documented in `docs/SPELLFORGE.md`.

- **Cold custom-mission save/replay identity.** Every current native save and
  replay carries a mandatory bounded descriptor for its mission basename,
  profile proto/map identity, and immutable asset source. Archive missions
  record exact mission/shared ZIP hashes and sizes, the selected nested RHM
  entry, and safe logical installed and/or distributed-cache locators; absolute,
  traversing, platform-ambiguous, control/invisible, and sentinel identities
  are rejected. Cold load verifies and mounts those exact bytes before engine
  construction and fails on missing or mismatched content. The package already
  embedded in a save/replay remains the sole Lua authority. Local and browser
  playback use a separate 64-MiB bounded isolated-worker lane that fits a
  maximum valid Spellforge package; ranked/server admission retains its
  stricter 16-MiB default and does not inherit local content trust.

- **Shipping dictionary rank permutation** (`convert_datadir
  --rank-dictionaries`, default on). Sprite dictionaries are reordered by
  tile-use frequency and all VQ indices rewritten to match at conversion time;
  invisible to the decoder, ~-2.9% on the RHS chunk bucket. Verified
  pixel-identical via `sprite_compression_probe --verify-shipping`.

- **VQ sprite context-model codec** (`robin_assets::sprite_codec`, library
  only — not yet wired into the shipping schema). Adaptive PPM + range coder
  over tile-index grids with optional cross-variant base coding; measures the
  full character corpus at 2.27x smaller than zstd-19. Integration design in
  `docs/COMPRESSION.md` (schema v7 section).

- **Touch camera gestures and native-refresh presentation**. Touch input now
  classifies taps, drags, double-taps, and two-finger transforms without
  leaking cancelled pointer actions into gameplay. World gestures support
  anchored pinch zoom, pan, bounded inertia, and UI/minimap exclusion, with an
  independent Gameplay toggle. A separate Graphics toggle presents and
  interpolates at the display's actual cadence while deterministic simulation
  remains fixed at 25 Hz; 60/90/120/144/240 Hz are covered without a
  hard-coded refresh-rate policy.

- **Level selection tree and campaign history.** Campaign progress is
  available through selectable Classic Map, prerequisite/progress tree, and a
  modal Sherwood Hall of Deeds exhibit grid. Every Rust campaign always
  retains immutable full-fidelity records for every attempt (including
  losses and practice replays) and derives totals/bests from those records.
  Only original-game saves are imported; their limited status/recent-mission
  data remains explicitly incomplete. See `docs/CAMPAIGN_HISTORY.md`.

- Item reliability rebalances are implemented as independent Gameplay
  settings. Direct apples can interrupt active swordfights; wasps acquire
  valid initial targets within 75 instead of 50 units; Will Scarlet's stone
  can use the sibling throwable base range 300 instead of its shipped 200;
  resistant VIPs/riders/Stuteley are skipped while a net catches other people
  in its original strict 40-unit circle; and outdoor non-VIP soldiers with
  authored beer value zero accept ale at minimum potency 20. Positive authored
  beer, net terrain crumpling, ally capture, and purse behavior are unchanged.
  Each rule defaults on independently; Original-parity sessions force them off.
- Ground-targeted stone noise distractions are implemented. With the
  independent gameplay toggle enabled, a real Stone projectile may target
  valid ground and emits one deterministic 240-unit noise stimulus on its
  terminal impact. Guards use the existing heard-noise search behavior. The
  command, target layer, replay, rollback, multiplayer, and quick-action paths
  remain authoritative; its additional impact cue is independently toggleable.
- Cloaking (implemented, optional): selected heroes whose sprite profile has
  the shipped cape rows can put the cloak back on with a rebindable key. The
  reversed original transition leads to a dedicated stationary Cloaked state;
  unaware distant hostiles are deceived, while remembered targets, ordinary
  line-of-sight after a reveal, and close scrutiny see through it. Acting or
  taking damage reveals the hero. Fresh profiles enable this; migrated
  profiles preserve original one-way cape behavior until enabled in Gameplay.
  Original replay construction always forces the feature off. The shipped
  human visibility routine has no character-specific detector and the unused
  animal runtime has no shipped mission instances, so the explicit authored
  detector seam remains disabled with a TODO for a future mod schema instead
  of assigning invented special senses. `cloak_art_audit` validates both cape
  rows for every declared PC profile: the full Linux data has 10/10 available
  and eligible tracks; the Leicester demo has 5/5 available tracks eligible
  (its CPF also declares five full-game profiles whose RHS files are absent).

### Additive hackable sprite mods

- Overlay mods can append soldier profiles through
  `Data/Configuration/profiles.patch.json` without replacing the
  retail CPF profile table.
- The sprite authoring tool can extrapolate one additional combat-stat tier
  from two adjacent retail tiers and emit concrete JSON Patch operations.
  This supports elite variants beyond the original black-guard ceiling while
  retaining each unit role's established progression.
- Readable soldier identifiers use normalized CPF filenames. When the retail
  CPF repeats a filename, hackable levels retain the original numeric identity
  with `<name>__<cpf-index>` (for example `archer05__47`) instead of silently
  choosing one record.
- Hackable RHS manifests explicitly select `rgba` or `legacy_color_keys` PNG
  semantics. Legacy green transparency and blue cast-shadow masks remain
  available to ambience-aware rendering instead of being baked into alpha.
- One overlay mod may expose multiple hackable missions through the
  `hackable_missions` array, and large character packs may opt into
  mission-scoped sprite loading.
- Native builds compile each hackable `.rhs.d` PNG tree into an atomic,
  zstd-compressed runtime cache beside its manifest. Cache hits reuse packed
  engine sprites and animation tables; manifest hashes and source file
  metadata invalidate stale caches automatically.

### Legendary enemy-placement candidate generator

- A deterministic generator (formerly `docs/legendary-enemy-proposals/generate.py`,
  which is not part of this repository; TODO: check it in or record where it
  lives) builds a browser-based mission review from a hackable datadir. It renders authored
  start facings, walkable surfaces, active and inactive walking paths, and
  before/after crops for each candidate.
- The generator accepts mission, surface, background, location, and output
  arguments. It resolves every used soldier profile through the hackable CPF
  profile table, classifies its gameplay role, and renders the profile's actual
  directional sprites. This supports full-game missions rather than only the
  Leicester demo roster.
- Candidate rules expand safe officer/subordinate groups with paired ranks,
  distribute cloned groups evenly along suitable patrol paths, reinforce
  established archer lines only on locally thin wall-like surfaces, create
  narrow guard posts, cover open route ends with sentry pairs, and find ground
  gaps outside both authored starts and complete active patrol corridors. The
  rejected guard-to-archer conversion rule is deliberately absent. Unsafe
  candidates retain their attempted coordinates and render as red-highlighted
  before/after diagnostics rather than being silently omitted. Ambiguous inputs
  that cannot produce one meaningful placement are listed separately as not
  placeable.
- Second-patrol candidates prefer a 50% cycle offset, then alternate outward
  one percentage point at a time through 40–60% until a valid formation is
  found. Eligibility requires the officer itself to own the group's one
  unambiguous path and start within 80 pixels of it; subordinate-only path
  references do not turn a stationary formation into a patrol. Each attempt
  aligns the officer-to-escort axis and authored facings to the target
  walking-path direction. The additional route frame fits the complete shared
  path while dimming the map and hiding unrelated paths, surfaces, and soldiers.
- Presets and scalar command-line options control patrol count and spacing,
  officer roster targets, stationary groups, guard posts, route-end sentries,
  wall-archer density, per-sector limits, and whole-mission reinforcement
  budgets. The full-game batch report keeps accepted, alternative,
  budget-limited, conflicting, and low-confidence candidates visible so a
  designer can audit what the rules attempted instead of only seeing winners.
- The `legendary` preset's reviewed selections are exported as an embedded
  runtime manifest. On Legendary difficulty, the engine deterministically
  clones the selected authored soldier profiles after loading live navigation
  geometry and before spawning mission entities. Every placement is resolved
  against the runtime walkable grid, authored soldier indices remain stable,
  officer subordinate lists are extended explicitly, and cloned patrols receive
  their own commander/follower ownership. Normal, Hard, and custom difficulty
  missions are unchanged; stale manifests fail mission startup loudly if their
  expected authored roster no longer matches the loaded mission.

## Todo

- **Multiplayer follow-ups**
  - Sign matchmaking announcements with the game identity key so a peer
    cannot advertise a game under another host's endpoint id.

- Add a method to unhorse horsed soldiers without killing them; no-kill runs
  are annoying with horses.
  - Add an option for Merry Men to knock people out instead of killing them.
- A freely walkable world-space Sherwood Hall of Deeds (the modal exhibit
  grid is implemented).

### Code Quality

- Finish moving legacy sentinels to typed runtime boundaries. Entity IDs,
  titbit IDs, layers, sectors, obstacles and AI patrol paths now use nominal
  handles and `Option` where absence is meaningful. Raw level-data structs
  still retain authored `0xFFFF` values, and a few animation/ammunition fields
  use the maximum value as real Original-game protocol. Convert remaining
  runtime fields only when their semantics are proven; keep asset-reader
  translation at the binary boundary instead of spreading sentinel checks.

## Removed

Post-port functionality that was built and then deliberately retired. The code
is recoverable from Git history.

- **Leaderboard account self-service flows** (commit 25cb60809, "retire dormant
  account flows"). The client no longer offers username changes
  (`username_challenge`/`update_username`), deletion of an owned run
  (`deletion_challenge`/`delete_owned_run`), abuse reports (`report_abuse`),
  player-profile lookup, or the browse screen's run/campaign-session detail
  and verified replay download (`persist_verified_replay`,
  `trigger_verified_replay_download`).
- **Sprite compression research probes** (commit b112f8141). The
  `sprite_probe_experiments`, `sprite_probe_rdo` and `sprite_probe_rle_dict`
  examples were retired after their experiments closed; `docs/COMPRESSION.md`
  records the findings. `sprite_size_bench`, `sprite_compression_probe` and
  the `scripts/sprite_compress_*.sh` helpers were later removed the same way.

## Not-Todos

These are intentionally out of scope. Do not move them back into `Todo`
unless the project goals change.

- **JPEG / TGA / BMP write support for the asset picture layer**. The game
  data path does not need general-purpose image import/export. Keep the
  runtime focused on the formats actually used by shipped assets and current
  tooling.

- **General legacy parser utilities**. Do not rebuild small ad-hoc text
  parsers unless a current asset or tool path needs them. Prefer structured
  formats and existing Rust crates for new tooling.

- **Archive mounting as a user-facing feature**. Loading from the configured
  data directory is enough for normal play and development. Extra mount-stack
  behavior only belongs in a tool if a concrete workflow needs it.

- **Editor-only picture operations**. Pixel blits, format conversion helpers,
  and save/info paths that only supported an external editor are not gameplay
  features. Add focused command-line tools instead if we need asset inspection
  or conversion.

- **Software-renderer parity**. wgpu rendering is the supported path.
  Rebuilding a complete CPU renderer is not a feature goal.

- **Unused platform abstraction layers**. Mobile, timing, and placeholder
  subsystem stubs should not be reintroduced as standalone compatibility work.
  Add platform code only when it directly supports a target we actually ship.

- **Motion blur / blind tunnel-mask cursor effects**. The apparent blur path
  was not a real gameplay-visible motion-blur feature. Keep the cursor work to
  explicit effects with current gameplay hooks.

- **Sniper zoom or gun-specific UI**. This game has no guns or sniper
  mechanics, so any zoom work should stay framed as camera readability,
  widescreen limits, or accessibility.

- **Bug-for-bug fidelity when it makes the game worse**. Keep deterministic
  behavior and mission compatibility, but do not preserve dead code, obscure UI
  quirks, or obviously unused systems solely because an older implementation
  had them.

## Reversible background patch animations

Gameplay options includes **Reversible Background Patches (Next Launch)**
(`gameplay_config.reversible_background_patches`, default `false`). Start a new
mission after enabling it. Animated patches that were authored as one-shot
remain active: triggering them again plays their transition backwards and
restores the original background, collision/pathfinding obstacles, sight
obstacles, occlusion masks, sectors, lines and connected door rights. Subsequent
triggers alternate both states. Existing reversible patches retain their normal
behavior; patches without a transition animation retain their original policy.

The policy is recorded in simulation configuration and resolved patch state,
so current-format saves/replays retain it. Original-parity mode disables it and
standard ranked policy does not admit it. Existing sessions keep their resolved
patch policy. Target callbacks remember the animated patch group they apply;
subsequent activations replay that group directly, bypassing script one-shot
guards without repeating mission messages or rewards. Locks and transitions
prevent repeated activation while a mechanism is unavailable or still moving.
Captured controls retain their usable sprite instead of accepting a queued
one-shot freeze pose (Lincoln's cut drawbridge rope otherwise becomes entirely
transparent and cannot be clicked again). Other targets retain their authored
animation behavior.
TODO: patches applied later by delayed script commands need authored trigger
metadata; only patch changes made during the target callback are captured.

This state changes the native formats to save 73, replay 31 and multiplayer
protocol 40. Earlier Rust saves/replays retain their original files but are
rejected by the existing strict schema checks; they are not silently migrated.


### Replay recording across arbitrary save loads

Replay schema 33 embeds the exact serde JSON save before post-load fixups when
loading a save without a reproducible timeline marker. This includes saves from
earlier sessions, asynchronous autosaves, and saves captured after commands in
the current frame. Playback restores engine, sound, and persistent game state
through the normal load path, without depending on the original save file.
Clean same-session saves still use compact load-back markers. Loading after a
terminal recording starts a new recording with the embedded restore boundary.
Save loads remain ineligible for ranked submissions.

### Ordered pending AI detectable mutations

Pending AI detectable additions, appends, removals, and entity removals share
one authoritative FIFO, preserving their emission order through save/load,
rollback, and reentrant owner boundaries. This changes persisted state and
state hashes, including snapshots with no pending mutations. The coordinated
native contract at this milestone was save 76, replay 34, and multiplayer
protocol 42; ranked admission and browser invitations require matching versions. Earlier
Rust saves/replays and peers are rejected by the existing strict version gates,
not silently migrated. Original-game save import and original parity trace
formats remain separate and unchanged.

### One-command production release

`scripts/release.sh` releases the leaderboard service (a Debian 12 container
build, then `ops/deploy.sh` on the VPS) and the web game (runtime staging,
full-corpus assembly, and `deploy-cloudflare.sh`) with one command. It supports
`--server-only`, `--web-only`, `--rebuild-datadir`, `--dry-run` and
`--ssh-config`. It refuses dirty trees and commits that are not the tip of
`main`. It rebuilds the Demo datadir only when the live datadir header differs
from `SHIPPING_DATADIR_VERSION`, logs every run, and prints the stage's
rollback commands on failure. `deploy-cloudflare.sh` gained `--datadir-only`
and `ROBINHOOD_PUBLIC_RETAIN`, so a public deploy keeps previously published
public objects. `scripts/test_release.sh` (tooling suite) checks the script
with stub tools.
## Crash and bug reporting

The native game queues Rust panic and fatal startup/game-loop reports under the
OS data directory's `robin_hood/reports`. Before game initialization, a separate
upload-only process starts from the same executable. It submits older reports,
then waits on a pipe held open by the game. Normal exit, panic, or abrupt process
termination closes that pipe; the helper uploads newly queued reports even when
the game cannot complete startup. It skips game initialization and the updater,
and never starts another helper. Replay-upgrade workers do not start uploaders.

Reports go to the leaderboard VPS at `POST /api/v1/diagnostics`, with connection
and request timeouts and no redirects. After game exit the helper drains bounded
batches, retrying failures after 1, 5, and 20 seconds. Reports remain queued when
offline and are marked submitted only after a matching receipt. A process lock
serializes helper and manual uploads; the OS releases it after a crash. Local
`uploader.log` records receipts and failures. If helper startup fails, the game
falls back to its background upload worker. The helper uploads captured reports;
it cannot capture failures that prevent the OS from starting the executable.

Open the in-game console (`~`) and enter `BUGREPORT description of the problem`
to submit a manual report. Submission status and the report ID appear in the
console. Reports include the engine commit, platform, panic backtrace, recent
debug log (up to 32 MiB), and active replay JSON files (up to 224 MiB decoded). Missing or
oversized replay attachments are explicitly reported. Logs and replays can
contain player names, local paths and gameplay.

Native game-thread panics suppress the secondary “terminated without publishing
an exit code” report only after the panic report is safely queued. Both graphical
and headless frame loops update the diagnostic mission frame.

The native script HTTP server tries successive loopback ports when its requested
port is already occupied (for example, 17640 → 17641). The startup log records
the actual listening address; `--http-server 0` still disables the server. Other
socket errors and exhaustion at port 65535 remain explicit startup errors.

Native reports also attach versioned `native-context.json`: the last observed
lifecycle stage, mission and timeline frame, headless/replay/multiplayer mode,
selected datadir and core overlay path, successfully mounted mod overlay paths,
GPU/backend/driver, process ID, uptime, and capture thread name. Context is
recorded as the game progresses; capture never queries the live engine. A busy
or poisoned context lock produces an explicit unavailable reason instead of
blocking a panic hook. Strings and overlay counts are bounded, and the context
shares the existing attachment budget. Mission/frame and replay tracking reset
on a new mission or return to the main menu. Raw launch arguments, invitation
credentials, and connection endpoints are not added to this attachment.

The VPS stores diagnostics privately, separately from ranked evidence. Its
existing operator bearer token protects list, detail and deletion endpoints:
`GET /api/v1/operator/diagnostics`,
`GET /api/v1/operator/diagnostics/{report_id}`, and
`DELETE /api/v1/operator/diagnostics/{report_id}`.
The latest 100 reports are listed. Identical compressed bytes share a receipt, allowing
safe retries. Admission limits are 10 reports per IP/hour, 100 globally/hour,
100 MiB compressed per complete report, and 512 MiB total stored payload. The
native client compresses JSON with zstd; browsers use their built-in gzip
`CompressionStream`. Compression happens before upload and size validation.
The VPS stores the original compressed bytes and counts that size toward its
storage budget. The server never decompresses diagnostic reports, either on
submission or download, and there is no decoded-size or decompression-memory limit.
Kind and build metadata come from client-supplied headers and are untrusted.
The detail endpoint downloads the original compressed attachment without
Content-Encoding, so browsers preserve its compression. Receipts hash the exact
uploaded bytes. Legacy uncompressed entries remain downloadable as JSON.
Entries older than 30 days
are removed during the next successful submission transaction.

Deployment requires database migrations 0003–0005 and a matching schema-version-5
VPS release. Existing nginx and Cloudflare API routing covers the new endpoints.

The browser toolbar's **Report bug** button opens a report form. Unhandled
JavaScript errors, rejected promises, Rust panic console messages and fatal boot
errors also queue reports. Failed reports retry on reload or when connectivity
returns. The IndexedDB queue holds at most ten pending reports and migrates old
localStorage entries; automatic capture is
limited to three reports per page load. Browser reports include logs and build
details, but do not yet include a replay attachment.

TODO: a native report form, browser replay attachments, native fatal-signal
minidumps, queue retention settings, and coherent replay snapshots. Rust panic
hooks do not capture OOM, SIGKILL or native fatal signals; a captured replay can
end in an incomplete write.


## Inline leaderboard replays

Replay launches use a disabled save store in graphical and headless modes. They
do not open the player's save index or autosave manifest, and reject persistent
save requests. Recorded save/load boundaries remain in-memory playback state.

Verified run pages include an embedded replay player with playback controls and a
full-page link. Full-game replays load hosted shipping data automatically. Each
recorded runtime has an immutable `datadirs/replays/v2/<build>.json` binding to a
content-addressed Full package; Demo play keeps its existing data selection.
Build Full data with `ROBIN_WEB_CONTENT_EDITION=full scripts/build_web_shipping_datadir.sh`,
then stage it with `wasm-www/scripts/add-full-replay-content.mjs` before publishing
the datadir corpus. The matching recorded browser runtime must also be published.
Reviewed fixes that change only the host can select a patched playback build;
game data and replay provenance remain bound to the recorded build. Runtime
`1f546fbb6547` adds save isolation, replay camera/audio controls, and corrected
speech handling to recordings from `1699bc12ffb8`.

Replay viewers can drag the world with the left mouse button, including while
paused. Pausing replay playback pauses music and resuming continues it. Browser
play starts with a free cursor and edge scrolling disabled; **Capture cursor**
enables pointer capture and edge scrolling, and Escape releases it. Embedded
leaderboard replays start in the free-cursor mode as well. These controls affect
presentation only and never produce simulation input in a replay.

- Full-game leaderboard “Any ruleset” is a combined browsing view across configured Full boards, with shared ranking and pagination. The legacy `full-any` URL remains supported; it is no longer a production submission board.

- Shipping conversion keeps cinematic video bytes in separate content-addressed files. Small per-movie references retain locale fallback in the boot data, and native video playback verifies and reads the selected file only when requested. The `split_cinematics` asset example migrates existing boot files without re-encoding mission assets; regenerate the web content manifest afterward.

### Leaderboard browsing and latest submissions

The leaderboard site defaults to the full game and combines full-game and demo missions in one numbered, grouped selector. Locations without a mission victory are excluded. Times show hundredths of a second, preserving every 25 Hz simulation frame. The first results page also shows the ten most recently verified public runs across missions, via `GET /api/v1/latest-runs`; anonymous uploads remain anonymous and removed runs are excluded.

The leaderboard home page puts recent submissions before mission rankings, offers submission instructions and mission launch links, and shows score and exact time together. `include_metrics=true` adds verified run metrics to leaderboard entries; existing clients retain the previous response shape. Browser mission links select the demo or hosted full-game content for the chosen build.

- Leaderboard run badges expand to show the game’s achievement names and rules. Run results appear above responsive replay viewers, and player profiles pair score/time personal bests with mobile cards. Older text-format run records remain viewable and downloadable, with playback explicitly unavailable.

### Standalone projection assets and discrete editor states

Descriptors may define independent static GLBs in `state_variants.initial` and
`state_variants.applied`, each with a `name`, descriptor-relative `model`, and
optional endpoint-specific `parts`. The Assets panel lists each endpoint separately;
only the selected GLB is loaded. Variant-qualified resource IDs allow both endpoints
in one document, with model and descriptor hashes checked on reload. This does not
provide animation. Supplemental `mission-*` parts identify their source through
`mission_profile` instead of claiming a sight-obstacle index; their local footprint
supports editor placement only. Game baking rejects supplemental mission models
except unchanged native initial previews verified against a pinned GLB and native
mission-file binding; those retain their existing game patch data. The legacy
baker still reconstructs original volumes rather than rendering refined GLBs.

The level editor’s Assets panel inserts exported projection assets from the current map as independent named groups. Saved documents pin descriptor/model hashes and reload the referenced GLBs; instances share rendering resources while retaining independent transforms. Explicit Initial/Applied group states switch endpoint visibility atomically, survive save/reload and undo/redo, and remap member IDs when duplicating a complete group. These are discrete states, not animation playback. Game baking rejects external asset references until their conversion is supported.

Explicit foliage materials keep physical leaf coverage separate from source ownership. Their RGBA texture supplies glTF MASK coverage at cutoff 0.5; COLOR_0 red stores the observed-source weight. The editor preserves alpha gaps in normal and source-only views and interprets the color attribute as evidence rather than a surface tint. The export contract supports double-sided cards or explicitly paired one-sided source/neutral cards, with optional unlit rendering. Vertex ownership requires geometry split at evidence boundaries; it cannot encode an arbitrary per-pixel ownership mask inside a triangle. Existing opaque projection materials retain their alpha-as-ownership behavior.

- Reviewed map assets can split one obstacle into independently selectable component parts. Exported component IDs retain canonical obstacle provenance and their own scoped collision footprint in map documents and standalone instances. Game baking rejects these split documents until component geometry compilation is supported.

### Map manifests and shared scene assets

Mission maps are `.rhlos-map.json` documents referencing pinned library assets through
`sceneAssets`. The editor requires the manifest; it no longer opens whole-map GLBs
or reconstructs missing documents during loading. Each reference includes its role,
model hash, and hashes for external geometry and image resources. Shared payloads
are stored once under content-addressed library paths and checked before loading.
Transforms, ownership, hidden parts, source obstacles, native patch bindings, and
reveal metadata remain in the document/assets. Opaque rendering uses stable names
instead of asynchronous material allocation order to keep overlapping surfaces
consistent across reloads and asset partitioning.

The refinement exporter produces map JSON and `map-assets/` directly from Blender's
separate glTF resources. Individual palette assets may still be GLBs. Staging,
material/ownership verification, browser audits, and guarded promotion consume the
manifest and install referenced resources before replacing the map document.
`pipeline/src/import-scene.ts` explicitly converts older published snapshots.
No mesh quantization, texture recompression, or collision regrouping occurs during
conversion.

Map asset navigation sockets support an explicit maximum height step at matching
projected edges. Both assets must permit the step; detached sockets remain separate.
This lets independently placed bridges and platforms share navigation while keeping
their authored receiving planes and height discontinuities. Offline reviewed recovery
can restore movement contours independently of receiving footprints and generates
the corresponding asset-owned collision clearances.

Asset movement surfaces and cutouts can retain fractional boundaries through boolean
assembly with `preserveMovementPrecision`. The final movement regions still use the
engine's integer grid. Ground recovery uses this to keep independently movable
cutouts aligned with terrain instead of rounding each piece separately.

- Editor terrain now uses bundled seamless grass, dirt, water and paving art synthesized from game map samples. Roads and rivers share the dirt/water art. Paved ground is selectable in the terrain panel and exports with stone surface material. A regeneration script records the donor crops and texture-synthesis CLI settings.

- Path authoring separates saved-path browsing from focused editing, keeps finish/cancel and width/elevation controls prominent, and groups texture, point coordinates, and wall tuning in collapsible sections. Footpaths and rivers can be drawn without an asset library.

- Compiled map resources can carry paired color/depth PNG states in disjoint image
  regions, with complete tables for overlapping patch combinations. The renderer
  selects both fields from current native patch state, including reset and timeline
  changes. Asset transitions can name local model appearances; editor export resolves
  fresh placement bindings, groups overlapping geometry and bakes the paired images.
  Purely visual transitions need no fabricated collision or door changes. Existing
  asset recovery, animated states and shared transitions remain incomplete.

- **Grid terrain authoring and map workspaces.** New maps start with named map-size
  presets, pixel dimensions, grid spacing and elevation. Workspace resizing retains
  all out-of-bounds content; export clips walking areas and materials to its frame.
  Terrain uses shared movable XYZ vertices, local subdivision, point materials,
  smooth texture blends and per-cell walking overrides. Roads have point widths
  and materials and conform to the edited mesh; rivers can derive channels without
  changing the control grid. Asset moves and terrain changes preserve manual height
  offsets. Camera controls provide cardinal headings, top view and quarter turns.
  Visible, outline and hidden asset display modes leave saved/exported content intact.
  The shared catalog includes 52 terrain/path/water designs and custom name/color
  materials. Wychford now has editable terrain, road paths and material areas;
  its conversion preserves placements and provides a reproducible height audit.

- **Terrain selection tools.** Draw separates Terrain and Paths modes. Terrain
  supports Shift-click multi-selection, Shift-drag rectangle selection, edge and
  cell dragging, and hover previews of every affected vertex. Group moves preserve
  relative offsets; Alt-drag moves horizontally and ordinary dragging changes
  elevation. Local subdivision leaves unrelated cells unchanged.

- Top, cardinal and 90° rotation camera buttons use the same smooth transition as the Game camera control,
  which now sits beside them in the viewport navigation bar. Camera clipping follows
  current terrain and assets after resizing, edits and previews without refitting the lens.
  Camera transitions interpolate orbit orientation and distance around their focus,
  avoiding inward cuts and jumps when another camera button interrupts a transition.

- New terrain grids use square world-space cells rather than square projected pixels.
  Terrain grid edges use thicker screen-space lines. Dragging an endpoint after
  picking an edge or cell moves only that vertex; explicit Shift selections move together.
  Height edits orient quad diagonals across isolated selection-boundary corners,
  persisting the resulting slopes in saves and exports.
  Terrain vertex picking uses a forgiving screen-space radius. Double-click refines
  the clicked cell or cells sharing the clicked edge/vertex. Right-drag retains
  camera rotation; Shift-drag adds a rectangle of vertices to the selection. Flatten
  levels selected vertices to their average height without changing their footprint.
  Delete removes selected vertices and reconnects the surrounding ground as one
  undoable edit, rejecting deletions that cannot form a valid surface.

- Editor modes in the title bar select matching left-side libraries and right-side
  inspectors: Assets, Paths, Terrain, and Mission. Search and preview-card controls
  are shared across assets, wall/fence presets, textured materials, and characters.
  Mission loading lives above the character library; View settings remains separate.
  Help follows the active mode, and map-only controls stay out of the map-selection title bar.

- Material libraries use prebuilt 128px thumbnails and draw only visible cards,
  avoiding full terrain texture generation when selecting a path. Roads and rivers
  initially filter to their respective material categories; other categories remain available.

- Asset and wall libraries share a parsed-model preview cache across mode switches.
  Recently used previews remain available within a 64-entry / estimated 128 MiB
  budget; visible cards pin their resources until released, and eviction disposes
  geometry and textures. Concurrent requests share one load.

- Paths mode shows all spline centerlines; click a line to select its path.
  Spline lines use the terrain grid's screen-space thickness and, together with
  their control points, remain visible above terrain, path surfaces, and assets.

- Path surface textures use half the previous resolution in each dimension;
  dragging uses a bounded 64×512 live material blend and restores the committed
  surface resolution on release. Source terrain/material pixels share a 64 MiB
  least-recently-used CPU cache while rendered textures retain independent ownership.
  River previews reuse the surface synchronized with their channel instead of
  generating it twice per pointer update.

- Double-clicking ground with a path selected inserts a control point into its
  nearest curve section, interpolating width, elevation offset, and material blend.
  Shift-drag adds enclosed points of the selected path to the selection. Dragging
  a selected point moves the selected set; clicking a point selects it alone.

- Library publishing supports metadata-only gameplay frames, hashed catalog batches,
  and exact-byte chunked delivery of large runtime models. The HTTP editor restores
  the complete asset gameplay catalog and verifies model chunks before loading them.
  Texture baking now repairs collapsed UV charts, fits packed layouts into one tile,
  and includes retained meshes outside the scene hierarchy.

- The asset library hides non-rendering gameplay helpers by default. Enable
  “Show gameplay helpers” to browse lighting regions, navigation frames and similar
  metadata assets alongside the visible scenery.

- The level editor’s Elevation lines overlay generates contours every 32 height
  units from modeled terrain, including spline shaping, and refreshes during edits
  and undo/redo. Maps without modeled terrain retain imported elevation lines.

- Terrain dragging coalesces previews per frame, reuses unchanged terrain sections
  and river-channel tessellation, and retains surface materials and textures.
  Height edits preserve river surfaces and only redrape road geometry.
  A CPU drag benchmark is available at `level-editor/app/tests/terrain-edit-benchmark.mjs`.


- Wall, river, and road paths have a Curved checkbox; disabling it joins controls
  with straight sections, including the editing spline and closed loops. Existing
  paths retain their smooth curves.
  Path dragging coalesces previews per frame, shares repeated wall deformation work,
  and prunes degenerate road-clipping fragments when rivers reshape the terrain.
  CPU benchmarks are available in `level-editor/app/tests/path-edit-benchmark.mjs`.
- Sunlight defaults to enabled at full shadow strength for new maps and maps without
  saved lighting settings. Explicitly disabled lighting remains disabled.

- The vertical translation gizmo appears automatically below a 20-degree viewing
  elevation. The manual Z toggle keeps it visible at other angles.

- Roads use a bounded terrain-sampling mesh during path and terrain drags, then
  restore exact ridge/channel conformance on release or cancellation. Road-only
  edits retain river-cut terrain and other unchanged roads. Attached placements
  reuse both the committed and preview channel meshes during height following. Exact road clipping
  avoids fragmenting ribbons against triangles that do not intersect them.

- Large-map path edits reuse the base terrain spatial index, query only nearby river
  cuts, and populate modified height-lookup regions on demand. Road fitting shares
  that index. Terrain mesh updates reuse batch keys and write directly to typed
  buffers without changing sampling detail. Run
  `node level-editor/app/tests/large-terrain-path-benchmark.mjs` for a size-scaling benchmark.

- The local asset library includes Long Wood Bridge from Sketchfab as a textured
  scenery asset, searchable under Bridge or Sketchfab. Credit: Horus Chen (embedded
  author), currently Kogeniku; CC BY 4.0. Source:
  https://sketchfab.com/3d-models/long-wood-bridge-e2b094603d0a44c8bf5a94a899e5c01c.
  `refinement/import_long_wood_bridge.py` reproduces the import from either verified
  download, preserving texture payloads and mesh detail while adapting scale and axes.

- The default “All refined levels” asset selection also includes Sketchfab imports. Multi-map PBR assets now receive lossy AVIF derivatives while retaining geometry, UVs, normals, and material parameters; independent normal/roughness channels use higher-quality 4:4:4 encoding. Long Wood Bridge includes a lossy model and lightweight preview.

- Prepared curtain-wall assets can carry compact model-derived collision caps and
  explicitly selected walkways. Export bends these definitions with the wall,
  joins repeated sections, and keeps parapets outside the walking area. Generated
  maps can route characters around inward area corners, including curved wall
  walks and bent platforms, without a precomputed navigation graph.
- Rising wall walks keep one continuous navigation region while retaining their
  individual height planes. Asset-local deck clearances prevent supporting caps
  from cutting holes in the route after bending; separately placed obstacles
  continue to block movement.
- Asset navigation sockets can opt into a minimum shared edge length, connecting
  differently sized walkways and placements shifted along a seam. Several neighbors
  can use disjoint spans; overlapping claims are rejected and detached edges stay separate.
  Seven Sherwood bridge/platform assets now support this placement rule.

### Riverbank designs in the level editor

River control points now carry separate left and right bank designs: plain,
small stones, big stones, mixed stones, small stones with plants, and vegetation.
Choose a selected point, selected section, or whole river in the Riverbanks
controls. Section changes set both endpoints and blend into neighboring sections;
inserted points retain the existing blend. Bank width is independent of water
width, and changing it does not scale the stones. No bank decoration preserves
existing maps and can clear a bank on either side.

Banks use synthesized Sherwood mission art, follow terrain and river edits, and
are included in saved maps and color/depth export. They are painted surface
decoration; their stones and plants do not add collision or alter ford navigation.

### Preserve gameplay during model publication

Staged model publication now rejects replacements that silently remove an
existing asset's gameplay definition. Reconcile that definition into the new
asset frames before publishing; this check also covers previously prepared
publication manifests. Best-effort map export remains available with warnings.
Ambient recovery accepts published draft review metadata and empty traversal
collections without treating them as additional sound behavior.

Sloped light recovery now bounds each attachment segment before neighboring
receiving floors. The local segment still follows its asset when moved or rotated,
without binding the asset to a preassigned runtime layer. Ambiguous light warnings
include the candidate sectors, layers and intersection heights.

Explicit shared appearance authoring can connect a secondary asset's visual state
to an existing control on another asset. Trigger geometry and receiving anchors
are converted into the secondary asset's own coordinates; collision and door
effects stay with their original owner. Matching local contacts assemble after
placement, and independent movement detaches them. This is an asset-authoring
operation, not automatic coupling by preview names.

Gameplay recovery now retains obstacle-state controls even when they change no
navigation polygons or door rights. Owned sight/mouse volumes switch with their
asset's appearance while persistent movement exclusions remain independent.
Incomplete or cross-asset physical ownership is reported instead of guessed.

Lincoln's plateau/terrace reveal is now authored as an appearance-only control.
It retains local activation geometry and a shared appearance contact without
inventing movement changes or door effects.

Async map export renders appearance changes using only intersecting map tiles.
It preserves the full-frame camera, tile boundaries, shadow framing and depth
normalization, while allocating only the affected region's output pixels.
Browser acceptance runs can report live progress with `TEST_PROGRESS=1`; timeout
errors include the last reported stage.

Nottingham's castle gateway now owns its three portcullis lanes and permission
control. Moving the courtyard floor independently no longer carries the gate
control away; the gate's appearance follows the same local control.

Nottingham's east gate tower now carries its own appearance-only reveal control,
with a local waypoint at the receiving floor's height. Its visual binding moves
with the tower independently of the neighboring gateway's door permissions.

Derby's keep hall and upper gatehouse now have asset-local reveal controls.
Their trigger locations and activation contours move with each asset; they do
not change navigation or door rights.

Derby's east-hall reveal now switches its owned sight volumes alongside its
appearance while keeping permanent navigation unchanged. Leicester's remaining
modeled reveals now have local controls too; the three-part moat reveal joins
through explicit contacts and separates when an asset moves away.

Best-effort export now reuses generated terrain across retries that omit
unavailable controls or traversal assemblies. Each new export regenerates terrain,
so subsequent editor changes remain authoritative.

Map compilation now retains asset-local changing barriers on ladders and climbable
walls. Placed fixtures verify collision, pathfinding and complete actor traversal
before applying the barrier, while closed and after reset, including a barrier
near an entrance whose inner endpoint moves during actor-clearance adjustment.

Spline masks now retain disconnected receiving-probe fragments after trimming.
Asset masks can author `receiverPolylines`; export intersects each fragment
independently and rejects competing receiving layers without connecting the gaps.

Spline lighting also preserves disconnected receiving-probe fragments after
trimming, retaining valid light regions and their ambience filters.

The woodland-bank asset now carries a bounded terrain attachment for its receiving
volume. Its two Wychford placements bind to their surrounding terrain while the
existing Leicester compilation remains identical; new rotated placements retain
the same finite attachment rule.
