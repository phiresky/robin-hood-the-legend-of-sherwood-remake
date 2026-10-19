# Map data: editor → game

The goal is **new maps that play well**, assembled from reusable assets and editor
terrain. Existing maps are regression examples, not templates whose original
records must be reproduced. Export reads only the editor document and asset-local
definitions. One-time extraction may restore missing definitions into assets.
Connections and runtime indices are rebuilt after placement, rotation and copying.

Independent floors inside another region's collision exclusion now receive
separate export-prepared layers, ordered so normal clicks reach the floor.
This fixes sixteen moved Lincoln keep entrance cases and an existing island-floor
regression without runtime movement changes. The Lincoln keep and cone-turret
doorway corrections are published: the installed library reaches 33/35 full-scene
entrance trips. Independently copied, rotated and raised three-asset assemblies
pass 80 terrain-click round trips and 128 within-room crossings. Browser reopening
preserves gameplay, color, depth and appearance states; the exact ZIP passes ten
entrance trips and sixteen room crossings. The supporting annex stays separate
and unchanged. Two neighboring plateau exclusions still block entrances.

A staged wall-floor correction now passes sixteen long click approaches across
separately placed slate-tower and curtain-wall assets, including rotated and raised
copies. The compiler aligns matched socket vertices before polygon union; this
adds no runtime work. The staged Lincoln export passes 34/35 entrance trips and
all sixteen stair-click routes. A subsequent east-slope floor candidate restores
the wall-climb landing: all eight full-scene climb routes and 32 routes through
copied, rotated and raised neighboring assemblies pass. Removing the slope
rejects all sixteen climb bindings. The candidates remain unpublished pending
the west-tower plateau repair and broader placement/browser review. See the
[shared-floor seam evidence](map-compilation-evidence.md#shared-floor-seam-survives-rotated-placement-2026-10-19).

Internal floor-hole seams now generate export-prepared connections between
separate receiving floors, with two complete-sprite native crossings verified.
The two York doorstep definitions are now published. Preserving prepared movement
precision and selecting supported approaches now lets the isolated 90-degree
stone-tower doorstep pass both native directions. The narrow timber doorstep at
that rotation has no full standing-box position on its physical floor. Both
houses pass all 32 building entry/exit round trips from their door approaches
across copied, rotated and raised placements. A stricter native test starts on
surrounding terrain and selects the actual entrance with a normal player click:
All 32 placements complete entry and return after preserving single-floor
precision, reconciling clipping-grid seams and sharing checked passage endpoints
on narrow landings. Candidate selection now requires endpoints on their assigned
receiving floor before choosing a crossing, fixing the 180-degree timber case.
Their browser ZIP reopens with identical gameplay, color and depth, loads natively
without base data, and passes all four archived terrain-to-room round trips.
York's descriptor pins are updated; model and texture bytes are unchanged.
Socket counts alone
are not traversal proof; see the
[prepared floor clearance](map-compilation-evidence.md#prepared-ordinary-floor-clearance-2026-10-14).

The completed ten-map sweep passes native construction. Its doorway-start
audit passes 169 of 277 entrances, with 100 denied routes and eight receiving
ownership mismatches; 99 denied routes have permanently unavailable exported
corridors, and one is blocked only by an authored lock. This compiler snapshot
predates the latest doorstep corrections and publication. See the
[ten-map audit](map-compilation-evidence.md#ten-map-native-construction-and-entrance-audit-2026-10-14).
Fresh Wychford export passes native construction and all 24 emitted entrance
round trips, confirming all four receiver mismatch fixes without runtime changes.
The latest main-library export also retains all 24 trips after the coplanar-layer
and Derby asset updates; see the [current regression evidence](map-compilation-evidence.md#fresh-wychford-regression-and-northern-plateau-authoring-2026-10-19).
The unsupported church-tower traversal remains omitted with a warning. The fresh published
York follow-up passes 46/95 doorway trips, with 46 denied routes and three receiver
mismatches. Explicit asset-local doorway structure references fix the two newly
blocked entrances in the assembled scene; all 32 isolated native placement trips,
64 negative terrain checks, and browser export/reopen checks pass.
The audit now includes the remaining trapdoor, which completes a native round trip.
The east and west courtyard lodges now pass in the assembled scene as well as
32 copied/rotated/elevated entrance trips. Their browser ZIP reopens identically,
loads without base data, and passes all four archived terrain-start entrance trips.

Fresh Derby, Lincoln, Nottingham and Sherwood exports all construct natively.
The published Derby ownership correction moves three tower entrances out of
the main-hall definition and links the split room in the editor. It preserves
assembled doorway positions and room memberships, passes 48 isolated placement
trips, and improves full-scene entrance results to 22/25 with the exporter now
rejoining valid receiving pieces after native-precision splitting. Both Derby
receiver mismatches are fixed. Independently moved tower copies pass 80 native
room crossings and sixteen disconnected-room checks. Mixed hall/tower placements
also pass 32 doorway trips and linked/unlinked checks. Their browser ZIP reopens
identically, loads without base data, and passes the exact-archive doorway and
room-crossing tests. The asset definitions and Derby's room link were installed
together; models and textures are unchanged.
The published Derby entrance batch passes all 25 doorway trips by declaring the
watchtower and central gallery's own passage structure. Explicit exterior
landing sockets also connect these assets to matching authored terrain: all 32
copied/rotated/raised terrain-to-room click tests pass, alongside 48 doorway-start
trips. The browser ZIP reopens identically, loads without base data and passes six
exact-archive click round trips. The affected Wychford asset-regression export
constructs natively and retains all 24 entrance trips. Definitions and both
saved-scene pins are installed; models and textures are unchanged. These results
do not establish full map parity.
Their current entrance results are Derby 25/25, Lincoln 33/35, Nottingham 43/69,
and Sherwood 5/5. Three Lincoln village doorway corrections improve its result
from 28/35 while retaining movement geometry. Those assets pass 48 moved-placement
room trips, browser export/reopen, native ZIP loading and six archived terrain-start
entrance trips. Remaining failures still require asset and connection work.
Sherwood's three doorway corrections retain walking geometry and pass 48 moved
placement trips, browser reopening and six archived terrain-start trips. Its five
entrance checks now pass; this does not certify all other map features.
Eight Nottingham village entrances now pass after local doorway declarations,
verified by 128 moved-placement trips, browser reopening and sixteen archived
terrain-start trips. One of Nottingham's remaining denials is an authored prison
lock with no permanent corridor block. The authoring audit confirms that no map
transition targets that entrance; its unlocked after-transition fields alone do
not authorize unlocking it. Mission-script behavior is outside this check.

Two further published Nottingham doorway definitions pass 43/69 full-scene entrance trips
and native construction, improving exactly the market-front brown house and
upper red house ground entrances. Four ground-start
gallery trips pass, and two additional platform-start trips pass when their
probes start on the elevated receivers. Ground access to those platforms is
not established by this fixture. Browser export/reopen preserves gameplay,
color and depth; the exact ZIP loads natively without base data and passes
all six entrance trips. Model and texture bytes remain unchanged.

The preceding saved-map checks recorded
290 prepared stair routes and 71 control apply/reset checks passing. The current
saved-York stair audit passed native construction and all 130 prepared stair
routes. Its older doorway failure counts are superseded by the current audit
above. See the [earlier York entrance audit](map-compilation-evidence.md#virtual-floor-boundaries-and-york-entrance-audit-2026-10-14).
An optional export trace now identifies exact doorway collision owners without
changing the descriptor. Two river-lane house corrections are now published after
32 placement/native trips, identical browser ZIP reopening and two archived
entrance trips. The current York counts above include this publication; see the
[collision diagnostics](map-compilation-evidence.md#exact-doorway-collision-diagnostics-2026-10-14).
Leicester now passes native construction and all 24 building
entry/exit round trips after published asset-local doorway corrections. Its
sixteen stair routes and twelve control apply/reset checks passed previously;
the doorway corrections leave their geometry unchanged. Wychford passes native loading
and four emitted stair routes; one unsupported tower lift is omitted with a warning.
An export-only doorway correction lets virtual-room entrances name their local
static structure without carving walking collision. Unrelated placed barriers and
changing obstacles remain checked. Six corrected assets pass 160 native room
round trips across copied, rotated and raised placements. Their rendered ZIPs
reopen identically, load without base data, and pass twelve archived entrance
round trips. The earlier aperture-edit candidate was discarded.
See the [remaining Leicester entrance corrections](map-compilation-evidence.md#remaining-leicester-entrance-corrections-2026-10-13).
See the [structural doorway fix](map-compilation-evidence.md#explicit-doorway-structure-2026-10-13).
See the [entrance regression](map-compilation-evidence.md#cottage-entrance-veto-regression-2026-10-13) and
[current saved-scene audit](map-compilation-evidence.md#current-saved-scene-audit-and-raised-receiver-fix-2026-10-13).

**Current definition inventory (2026-10-13):** `gameplay-coverage-lLsDz7` checks
1,286 indexed assets and finds zero missing definitions among placed assets in
the saved scenes. Only two background assets that cannot be inserted as objects
lack definitions; all insertable assets now have definitions. This checks library
definition presence and index hashes, not scene pins or gameplay completeness.
Definition presence does not certify feature completeness or full-map parity.
The green timber house draft now preserves its permanent structure/counter and
independent removable awning. Door-height placement passes 64 native room round
trips across copied/rotated/elevated state combinations. Its ZIP reopens identically,
loads without base data, passes GPU state/reset checks and eight archived room
round trips. Rear texture quality, actor activation and detailed contact remain
under review; see [the green-house checks](map-compilation-evidence.md#green-timber-house-publication-2026-10-13).
The north display-piece draft is published after independent removal/reset checks,
permanent-neighbor collision checks and 32 full-character walks. Its ZIP reopens
identically, loads without base data and passes GPU state/reset checks including
thin depth changes. Actor activation and detailed contact remain open; see
[the display checks](map-compilation-evidence.md#independent-north-display-piece-draft-2026-10-13).
The standalone Nottingham hall now has a published local-definition draft.
Separate hall/tower placements pass 32 room entry/exit round trips and 32 stair
routes after authoring doorway apertures on each owning asset. Other entrance
support and actor-triggered reveal/contact validation remain open. Its rendered
ZIP reopens identically, loads without base data, passes GPU state/reset checks,
and retains all four archived room round trips; see
[the hall checks](map-compilation-evidence.md#standalone-hall-restoration-candidate-2026-10-13).
A published combined Derby keep draft now reassembles all 71 parts' local
gameplay from its four components. It preserves floors, stairs, rooms, masks,
lights and sounds. All forty native stair routes now pass after static handoff
preparation was narrowed to avoid neighboring entrances' internal routes. Elevated
outside entrances now bind in authored terrain fixtures. A replacement model
now preserves all 71 component parts and both independent reveal identities;
its 37° rendered ZIP reopens with identical gameplay, color and depth. With
surrounding terrain, both reveal controls and all four combined visual states
survive ZIP reopening. The four-rotation entrance audit now passes all 48 routes:
export prepares an actor-sized outdoor approach, and static load preparation
prevents duplicate ground-plane transfers along building exits. The wider slopes
still cover three low ordinary passages. Actor-triggered reveal behavior remains
unverified. A compiler fix
now prevents receiving segments from binding to floor holes when controls are
allowed inside collision; it adds no runtime work.
The combined draft includes current compressed browser and preview models, and
Wychford's saved reference has been updated. Remaining limitations are retained as
asset warnings; see [the keep candidate](map-compilation-evidence.md#combined-derby-keep-authoring-candidate-2026-10-13).
The standalone Nottingham watchtower now has a published gameplay draft: two
copies across four rotations and two elevations retain their collision and upper
floors, and all 48 sampled native routes pass. Access must come from surrounding
assets. A two-copy rendered ZIP now reopens/rebakes identically and loads without
base data; Robin completes both directions across each exported roof and stops
on the correct receiver. Lighting and rendered character contact remain under review. See
[the tower checks](map-compilation-evidence.md#standalone-nottingham-watchtower-gameplay-2026-10-13).
Fragment 102 now has an independent activation draft: absence leaves its footprint
walkable, activation blocks it, and reset restores walkability. Eight moved/copied
exports pass 16 native apply/reset cycles and 96 route probes. The assembly retains
its separate permanent exclusion. The published two-copy ZIP now reopens/rebakes
identically, loads natively without base data, and passes independent GPU
color/depth state/reset checks. Actor activation and animated contact remain open; see
[the fragment checks](map-compilation-evidence.md#independently-activated-fragment-102-2026-10-13).
The standalone York canvas screen now has independent raised/lowered gameplay.
Eight placement variants pass sixteen native apply/reset cycles, 48 route probes
and sixteen complete-sprite walks beneath the raised state. Its published ZIP
reopens/rebakes identically, loads without base data and passes independent GPU
state/reset checks. Actor-triggered activation and visual refinement remain open;
see [the screen checks](map-compilation-evidence.md#independent-york-canvas-screen-states-2026-10-13).
York tavern furniture now has independent removal gameplay. Eight placement
variants pass sixteen removal/reset cycles and 144 native boundary-route checks.
Its two-copy ZIP passes editor round trip, native loading and GPU state/reset
checks. Robin completes 96 walks after removal across all placement variants,
plus twelve on the exact published archive. Actor-triggered activation and rendered
contact remain open; see
[the furniture checks](map-compilation-evidence.md#independent-york-furniture-removal-2026-10-13).
The stone-house awning also has independent raised/lowered gameplay: sixteen
native state cycles, 48 boundary-route checks and sixteen complete-sprite walks
pass across eight placement variants. Its two-copy ZIP passes editor round trip,
native loading and GPU state/reset checks. Actor-triggered activation and detailed
rendered contact remain open; see [the awning checks](map-compilation-evidence.md#independent-stone-house-awning-states-2026-10-13).
The southeast and west canopies have independent raised/lowered drafts, with
32 native state cycles, 96 boundary-route checks and 32 character walks across
their placement variants. A combined four-copy ZIP passes editor round trip,
native loading and all sixteen GPU control combinations; archived movement checks
also pass. Actor-triggered activation and rendered contact remain open; see
[the canopy checks](map-compilation-evidence.md#independent-southeast-and-west-canopy-states-2026-10-13).
The north cream canopy preserves its permanent part while swapping raised/lowered
pieces. Eight placement variants pass native state checks, including permanent
sight retention, and sixteen character walks. Its ZIP passes editor round trip,
native loading and GPU state/reset checks; actor-triggered activation and rendered
contact remain open. See [the mixed canopy checks](map-compilation-evidence.md#north-canopy-with-permanent-and-stateful-parts-2026-10-13).
The southeast green canopy retains permanent ground collision while its panels
change state. Eight placement variants pass sixteen state cycles, 144 route checks
and sixteen character walks through the free space. Its ZIP passes editor round
trip, native loading and GPU state/reset checks. Actor-triggered activation and
rendered contact remain open; see [the ground-prop checks](map-compilation-evidence.md#green-canopy-with-permanent-ground-collision-2026-10-13).
The timber-house awning preserves its permanent prop across raised/lowered states.
Eight placement variants pass sixteen state cycles, 144 route checks and sixteen
character walks. Its two-copy ZIP passes editor round trip, native loading and
independent GPU state/reset checks. Actor-triggered activation and rendered contact
remain open; see [the timber-awning checks](map-compilation-evidence.md#timber-house-awning-with-permanent-prop-2026-10-13).
The pottery canopy now also retains initial-state ground blocking alongside its
three permanent props. Eight placement variants pass sixteen state cycles,
384 boundary-route checks and sixteen character walks. Its ZIP passes editor
round trip, native loading and independent GPU state/reset checks. Actor-triggered
activation and rendered contact remain open; see
[the pottery-canopy checks](map-compilation-evidence.md#pottery-canopy-with-initial-and-permanent-ground-props-2026-10-13).
The red canopy retains its permanent walkable roof and jump sockets while swapping
grounded pieces. Eight placement variants pass sixteen state cycles, 336 route
checks and 128 character roof walks across all control combinations. Its ZIP
passes editor round trip, native loading and GPU state/reset checks. Connected
jump traversal, actor-triggered activation and rendered contact remain open; see
[the red-canopy checks](map-compilation-evidence.md#red-canopy-with-persistent-walkable-roof-2026-10-13).
The long cream canopy likewise retains its permanent roof and jump sockets while
swapping grounded pieces. Eight placement variants pass sixteen state cycles,
528 route checks and 128 roof walks across all control combinations. Its ZIP passes
editor round trip, native loading, GPU state/reset checks and sixteen fresh roof
walks. Connected jump traversal, actor activation and rendered contact remain open;
see [the long-canopy checks](map-compilation-evidence.md#long-cream-canopy-with-persistent-walkable-roof-2026-10-13).
The red/stepped-green canopy connection now passes 160 native player-click jumps
across independently inserted, socket-aligned copies, rotations and elevations.
Moving the partners apart removes their jumps with explicit warnings. This
certifies that particular connection, not every retained socket. The stepped-green
canopy now has an explicit ground datum, preserving its roof elevation during
insertion without changing saved placements. The default-height connection checks
pass all 160 native jumps, and a fresh standalone ZIP passes editor round trip,
native loading and four character roof walks. See
[the placement fix](map-compilation-evidence.md#stepped-canopy-insertion-ground-datum-2026-10-13) and
[the connection checks](map-compilation-evidence.md#connections-between-independently-inserted-canopies-2026-10-13).
The Nottingham dormer house now retains its mask, roofs and interior alongside
independent front-prop removal. An authored threshold clearance fixes entrance
authorization: sixteen moved/copied native entry/exit round trips pass, along with
state and boundary checks. Its ZIP passes editor round trip, native loading, GPU
state/reset checks and fresh entrance traversal. Explicit roof clearances now
prevent enclosing support volumes from fragmenting the usable roof. All 256
character roof walks pass across four rotations, two elevations and every control
combination; the updated ZIP also passes round trip, native/GPU checks and 32
fresh archived roof walks. Independently inserted curtain/dormer assemblies now
pass 160 ordinary-click jumps; dormer/stone assemblies pass 120 ordinary-click
jumps plus 40 hidden-click jumps whose higher-roof occlusion is verified
separately. Both connected assemblies now also pass rendered ZIP round trip,
empty-base native loading and GPU appearance checks, with fresh archived jump
traversal reproducing the same ordinary/hidden-click behavior. Detaching partners
removes the connections with warnings. Connections
between roof planes, broader offset landing coverage and animated rendered
contact remain open. See
[the connected archives](map-compilation-evidence.md#rendered-connected-roof-archives-2026-10-13),
[the inserted roof connections](map-compilation-evidence.md#independently-inserted-dormer-roof-connections-2026-10-13),
[the roof clearance checks](map-compilation-evidence.md#dormer-roof-support-clearances-2026-10-13) and
[the house checks](map-compilation-evidence.md#independent-dormer-house-with-usable-entrance-2026-10-13).
The northwest timber house now also has independent prop removal and its local
interior. Its front prop initially blocks entry; removing it opens only that copy's
door. Across four rotations, two elevations and every control combination, 32
complete entry/exit trips and 32 expected denials pass. Its ZIP passes editor round
trip, empty-base native loading, GPU state checks and fresh archived entrance-state
tests. Roof/mask authoring, actor-triggered removal and rendered contact are not
certified. See [the timber-house checks](map-compilation-evidence.md#independent-northwest-timber-house-and-entrance-states-2026-10-13).

**Current Wychford export (2026-10-12):** `saved-map-exports-ab3hoH` completes
in 825 seconds, including final stair-clearance preparation. This verifies the
convex-region duplicate-comparison correction against a complete fresh export;
the earlier baseline failed after roughly 52 minutes. Native construction and
all 98 sampled ordinary routes pass. The full 204 MB rendered ZIP now reopens
and rebakes identically, loads without base-game data, and passes fresh native
checks of 98 ordinary routes and four stair routes through both prepared and
player-click dispatch. Its eight appearance-control combinations pass native
GPU color/depth transition/reset probes. Animated character compositing and
broader gameplay coverage remain open. The church-side-tower connection
is still omitted because its outside endpoint has no matching-height support.
See [the completed export](map-compilation-evidence.md#corrected-wychford-export-completion-2026-10-12).
See also [the rendered archive](map-compilation-evidence.md#full-wychford-rendered-archive-2026-10-12).
An exact-integer clipping patch reduces one measured terrain-generation run
from 183 to 50 seconds with identical output; 289 focused tests and the editor
build pass. A subsequent exact-product filter preserves the same terrain output
in a 41-second run and passes 290 tests; full-export timing for that filter is
not measured. Export performance still needs improvement. See
[the clipping profile](map-compilation-evidence.md#exact-integer-terrain-clipping-profile-2026-10-12).

This file also retains historical investigations. Results that depended on the
retired physical-navigation solver do **not** establish current traversal parity;
assets need verification against prepared-graph movement. The current architecture
and its remaining runtime work are tracked in [the runtime inventory](map-runtime-inventory.md).

**Fresh cross-map checks (2026-10-12):** Derby passes native construction, 144
ordinary route samples, 28 complete-sprite stair routes and all twenty
posture-aware jump clicks. Leicester loads and passes 162 ordinary route samples,
but only 140/230 offset jump-click probes pass. All ninety failures involve the
great keep's internal connections. Thirty-two available interior-click probes
pass; fourteen other directions have no conservative supported probe and remain
unverified. These are traversal findings, not a new rendered ZIP certification.
See [the fresh checks](map-compilation-evidence.md#fresh-derby-and-leicester-traversal-checks-2026-10-12).

**Full Derby archive (2026-10-12):** the current 25 MB ZIP reopens/rebakes
identically and loads without base-game data or mission content. Fresh tests of
its archived descriptor pass 152 ordinary routes, 28 stairs through prepared and
player-click dispatch, and twenty posture-aware jump traversals. GPU appearance
checks pass all 32 control combinations. The lower-courtyard black shelter is
fixed by restoring its draft material's unlit flag; a fresh browser round trip
and native ZIP load pass with identical gameplay and depth data. Animated
character compositing and broader interactions remain open. See
[the archive evidence](map-compilation-evidence.md#full-derby-rendered-archive-2026-10-12).

**Current Croisement checks (2026-10-12):** all three current scene exports load
natively and pass 16/6/24 ordinary route samples. Croisement01 passes all 130
jump probes; Croisement02 covers all 40 with the appropriate helper posture.
Croisement03 now passes 100/100 after deriving one asset's receiver from its
existing walkable surface. The published change improves rotated/raised copies
from 132/160 to 156/160 with no regressions; the four remaining checks pass with
hidden-floor selection. The combined 160 checks comprise 144 actual traversals
and sixteen expected occlusion checks, not 160 traversals.
These full-map exports include concurrent local scene edits. The corrected asset's
two-copy, rotated/raised browser ZIP now reopens and rebakes identically, loads
without base-game data, and retains twelve archived traversals plus eight
occlusion checks. Five native captures with an actual guard atlas frame now provide
manually reviewed static upper-platform, edge and foreground compositing checks;
animated contact and full-map rendered ZIP fidelity remain open. See
[the captures](map-compilation-evidence.md#native-platform-character-captures-2026-10-12)
and [the receiver correction](map-compilation-evidence.md#croisement03-authored-landing-receiver-2026-10-12).

The published restoration of four asset-local keep movement contours improves Leicester
to **226/230** offset jump clicks with no regressions. All 46 interior directions
now have probes: 45 pass ordinary selection and one passes hidden selection.
Eight rotated/elevated placements improve from 722/1,360 to 1,240/1,360 passing
offset probes, also without regressions. Four full-scene and 120 placement
findings initially remained. Follow-up selection diagnostics show that all four
full-scene cases select a neighbouring roof normally and pass the intended lower
jump with hidden selection. Hidden selection also resolves 74/120 placement
findings; 46 remain open. Rendered ZIP coverage at every rotation is still outstanding. See
[the clearance recovery](map-compilation-evidence.md#keep-movement-clearance-recovery-2026-10-12).

A wider placement sweep now verifies at least one complete interior-click route
for all 272 directed connections. Of 2,448 start/destination combinations, 2,310
pass ordinary selection and another 132 pass hidden selection. Six overlapping
target coordinates at 37 degrees remain unresolved; this establishes sampled
reachability without claiming every click works.

The published keep's two-copy rendered fixture now passes ZIP reopening and
rebaking, including fifteen appearance states. Its 79 MB archive loads without
base-game data and retains the editor document with no mission content. All
68 sampled jump directions from the archive traverse: 66 ordinary clicks and
two hidden selections. In-game actor/depth compositing remains unverified.
See [the archive evidence](map-compilation-evidence.md#published-keep-rendered-archive-2026-10-12).

Fresh Lincoln passes native construction, 182 ordinary route samples and all
100 jump-click probes. Stair gate discovery now passes all 16 routes after
including ordinary passages touching stairs in the existing load-time receiver
cache. All authored connections remain. No new per-tick geometry work is added.
Export now keeps quantized ordinary endpoints on their receiving surfaces and
prepares ordinary stair approaches before recomputing passage states. The
four-angle diagnostic passes all 48 forced routes and 48 player clicks. Fresh
zero-elevation 90-degree and raised zero-degree assemblies pass all 24 forced
routes and 24 player clicks. Fresh Derby/Lincoln regressions retain all 44
stair-click routes, 326 ordinary route samples, 100 Lincoln upright jump probes
and twenty Derby posture-aware jump probes.
All eight placements now pass 96/96 forced routes and 96/96 player clicks
after integrated passage preparation, including raised 180-degree coverage.
All 64 missing/raised-neighbour cases are rejected. The rotated/raised browser
ZIP reopens and rebakes identically, including fifteen appearance states, and
loads without base-game data. Its archived geometry passes all twelve stair
player-click routes. In-game actor/depth compositing remains unverified.
See [the Lincoln evidence](map-compilation-evidence.md#fresh-lincoln-traversal-checks-2026-10-12).

**Installed Lincoln drawbridge connections (2026-10-13):** the main library now
restores both deck floors and all six passages, with the plateau's receiving
coverage limited to three supported top regions. The metadata retains the earlier
two-copy checks at four rotations and two elevations: 96 open round trips, 192
closed/reset denials, 576 directed gate searches and 576 state-aware player clicks.
Sixteen missing/raised-neighbour cases reject unsupported plateau entrances.

The verified full-map browser bundle `lincoln-rendered-library-9FBLqH` reopens and
rebakes identically through 45 appearances and loads without base data. It contains
these drawbridge definitions but predates the separate step/turret repair below.
Fresh installed-library export `saved-map-exports-h7ZuCA` combines both repairs
and is byte-identical to the native-tested candidate: 192 ordinary routes, eighteen
stairs, 100 jump clicks and all six drawbridge passages across open/closed/reset
states pass. Fresh complete-character checks of that installed export also pass
all eighteen stair routes and eight climb routes. The combined full-map rendered
round trip now passes in `lincoln-rendered-library-N7St4K`: the 170 MB ZIP reopens
and rebakes all 45 appearances identically, loads natively without base data, and
contains the exact previously tested gameplay descriptor.
Sampled bank seams retain gaps below one game unit, and animated
contact remains unverified; these are draft limitations, not full-parity claims.

**Installed Lincoln step/turret repair (2026-10-13):** both reusable assets are now
in the main library with fresh derivatives and updated saved-scene pins. The step
mesh agrees with its walking plane within 0.000073 units. The turret retains its
wall recess and supplies an actual through-opening, supporting floor and static
collision volumes; 420,810 sampled occupancy checks match its mesh. A 42-unit
opening clears the native movement footprint swept through a 40-unit standing
visibility envelope across all tested placements. This is not a complete sprite
animation clearance proof.

The final two-copy bundle, rotated 37 degrees and raised 40 units, passes rendered
export/reopen/rebake, native loading without base data and all four archived stair
directions. The sixteen placement fixtures pass 32 directed stair clicks and 32
missing/raised-landing rejections. Fresh full Lincoln passes 182 ordinary routes,
18 stair directions (including the repaired step), eight climbs and its jump-click
audit. The combined full-map rendered archive is now verified above; in-game
animated character contact remains separate from that bake/reopen check.
Floor texturing and animated contact remain draft warnings. General lift-approach
clearance still needs broader export-time review; no runtime solver was added.

**Sherwood stair recovery (2026-10-12):** the exporter now prepares short-stair
clearance from connected landing support, with matching-height, collision and
state guards. The published oak-platform footprint passes all 36 scripted
routes and 36 player clicks across nine rotations and two elevations.
Fresh saved-map export also passes both stairs, fifty ordinary routes and ten
jump clicks. A rotated/raised two-copy browser ZIP reopens and rebakes identically,
loads without base-game data, and passes all four archived stair routes with
both scripted and player-click dispatch. No wider asset geometry or runtime
solver is needed. Mesh contact and in-game actor/depth compositing remain open;
earlier authoring experiments stay unpublished.
See [the investigation](map-compilation-evidence.md#sherwood-stair-clearance-investigation-2026-10-12).

**York jump checks (2026-10-12):** the latest full-map audit passes all **720
checks: 709 traversals and eleven occluded lower-click cases**. The gable house's
roof selection region now follows its existing walkable surface. The published
asset passes 160/160 offset clicks at four rotations, two elevations and two
copies (previously 80/160). Replaying the same 456 interior clicks before and
after gives 12 normal-selection improvements and zero regressions. All 456
routes now have verified coverage: 384 normal clicks and 72 requiring the
existing hidden-floor selection mode. The rotated/raised browser ZIP also
reopens and rebakes identically, loads without base-game data or mission
content, and passes all 36 archived routes (18 normal, 18 hidden-floor).
Stock placement success does not clear wider click coverage or all placements.
See [selection-region evidence](map-compilation-evidence.md#gable-roof-selection-region-publication).
See [interior-click evidence](map-compilation-evidence.md#york-interior-jump-clicks-2026-10-12).

A fresh export exposed a receiving-plane error in joined-stair preparation:
volume vertices beyond the three native plane anchors were incorrectly required
to be coplanar. The exporter now uses the native receiving-plane definition;
eleven focused geometry tests pass. Fresh York export `saved-map-exports-Q7aNRO`
now completes and constructs natively: 162 areas, 1,234 receiving obstacles,
254 doors, 72 jump pairs and 269,206 grid blocks. All 130 stair player-click
routes and 286 ordinary route samples pass. The 720 jump outcomes exactly match
the earlier audit, including its thirteen failures. Central-south roof
experiments remain unpublished: an inset improves isolated offset clicks from
68/160 to 104/160, but 90/180-degree cases still need investigation.
Its interior-click sweep passes 256/288 probes; all 32 failures are at 180 degrees,
including incorrect ground-height landings. The main roof is almost edge-on at
that rotation (world-to-screen plane determinant approximately -0.001856), making
integer endpoint rounding significant. Replacing endpoints with unsnapped values
alone still fails all forty offset probes at that rotation; this does not justify
a format change or a runtime solver.

Receiver review `jump-integer-support-mom6hU` finds zero of five sampled points
on the rotated edge supported by either assigned receiver. Fifteen diagnostic
integer-span replacements (`york-integer-span-diagnostic-ns72f47a`) retain the
existing format and engine. Five pass all eight sampled interior-click routes
in both directions (`interior-jump-clicks-CeCWKa`); overall 100/120 pass. Only
11/150 perpendicular-offset probes pass, so these are representation experiments,
not a publishable fix. They establish that integer endpoints can represent a
working connection here. Automatic selection, clearance checks and broader
placement verification remain necessary before changing export behavior.

A geometry-driven search now rejects endpoints without full upright footprint
clearance, preserves opposite equal integer spans and prefers less height
variation. Its first-ranked candidate passes **160/160 offset traversals and
288/288 interior clicks** across four rotations, two elevations and two copies
using the published central-south assets. This improves the isolated published
baseline of 68/160 offset traversals without requiring the staged roof edits.
The search remains a diagnostic tool. Complete-span and motion-state guards
are now implemented and tested, and flight checks include every solid state
while retaining inactive receiving planes. The flight audit reports solid roof contact
on fifteen of 24 reviewed candidates; nine others have sloped heights it cannot
verify. Movement success alone does not clear those findings. Flight clearance,
production integration and regression checks on all connections remain open.
See [the search evidence](map-compilation-evidence.md#supported-integer-jump-span-search-2026-10-12).

The central-south houses now include their missing opposite roof faces, derived
from asset-local geometry and published with updated York scene pins. All 84 mesh
samples have support, with height differences below 0.087 units. Export
`saved-map-exports-Hvo6ax` passes native construction, all 130 stair routes and
288 ordinary routes. Its original jump audit improves five cases with zero regressions.
The subsequent execution-distance audit correction clears three false landing
failures without changing the map or runtime: projection may rewrite the facing
goal after movement, so the audit now observes the actual step distance before
that rewrite. Collision, displacement and completion checks remain required.
Repositioned pair checks improve from 68/160 to **112/160**, with 44 improvements
and zero regressions. Arbitrary placement remains incomplete. A native trace of
the diagnostic endpoint replacement also confirms anchor penetration up to
14.096 units despite successful arrival; that replacement remains unpublished.
See [roof-face and trace evidence](map-compilation-evidence.md#york-missing-roof-faces-and-observed-flight-contact-2026-10-12).

The published central-south roofs now pass **288/288 interior-click routes**
across four rotations, two elevations and two copies with unchanged endpoints.
Offset probes remain 112/160: 32 requested landing points lack receivers and
16 clicks miss the jump overlay. The 37-degree/raised browser archive reopens
and rebakes identically, loads without base-game data or mission content, and
passes all 36 sampled archived routes. Its compiled geometry matches the
isolated placement except for warnings about other disconnected connections.
Fresh tracing of those 288 passing interior routes records 72 airborne
anchor/solid contacts, up to 2.921 units deep. A one-unit takeoff shift clears the
original orientation but worsens other rotations and remains unpublished.
Export now checks the final rounded landing spans against their receiving-face
coverage after partitioning. It reports both unsupported copied connections at
180 degrees at each tested elevation, preserving the connections for best-effort
review. This checks coverage, not flight clearance or animated contact.
Seventeen actual recorded jump poses now have native GPU captures at 37 degrees
and elevation forty, using five animation frames and the recorded landing sector.
Inspected takeoff/flight/landing frames show no obvious clipping corresponding to
the first small anchor contact. This limited visual check does not clear broader
flight-envelope warnings, animated-contact coverage or offset-click limitations.
See [central-roof archive evidence](map-compilation-evidence.md#published-central-roofs-and-archive-verification).

The published west-lane roof correction passes all **72 interior-click traversals**
across nine angles, two elevations and two copies. It restores the mesh-supported
roof outline and moves the takeoff inward. All 18 exports construct natively;
56/360 perpendicular-offset probes still fail. That earlier full York audit had **13/720 failing
offset probes**, down from 28, with no regressions: the compiler clearance fix
repairs ten golden-house probes and this roof correction repairs five west-lane
probes. All ten selected full-map interior clicks pass. Rendered traversal and
the remaining wider click-coverage failures remain open; the asset retains its
draft status.

Its published 180-degree/elevated browser fixture now passes lossless PNG/ZIP
packaging, editor reopening and native loading without base-game data. Complete
asset definitions revealed another gap: both copied internal raised-lane return
jumps stall after landing (6/8 directions pass, including all four repaired
cross-asset directions). The following correction addresses those returns;
the isolated 72-click result does not cover every jump owned by the two assets.

The published correction to those internal jumps passes 360/360 internal offset
probes and 144/144 complete-asset interior clicks across nine rotations and two
elevations. It derives matching level spans from the lane's existing receiving
surfaces; no runtime change or replacement terrain is involved. Full York
retains exactly the same thirteen previously recorded failures, with no regressions.
An expanded internal-jump sweep passes 1,740 normal clicks and sixty explicit
hidden-surface clicks across 1,800 start/destination combinations. Its fresh
rendered ZIP now reopens identically, loads without base-game data, and passes
all eight complete-asset jump directions. Multi-character traffic and in-game
rendered actor/depth compositing remain unverified.

**Subpixel rotated floors (2026-10-12):** simple folded floors now receive
walking clearance before integer normalization can erase them. York's copied
two-house fixture retains both jump connections at all eight placements,
including 37 degrees. Native traversal improves from 40/120 tested probes plus
two omitted exports to 60/160 probes, with no previously passing probe regressing.
The remaining 100 failures and full-map verification remain open. This changes
export preparation only; the staged York edge edit remains unpublished.

The same preparation now covers nearly edge-on floors whose projection does
not reverse. A 100-unit floor rising 99.5 units previously exported a movement
polygon with no room for a walking footprint; the regression now verifies usable
clearance as well as receiving geometry on both sides of the projection limit.
All 180 focused compiler/clearance tests pass. York's west-lane candidate remains
at 60/160 traversal probes; this fix does not resolve its concave roof coverage.

**Folded-roof export preparation (2026-10-12):** the compiler now prepares
world-oriented joins, projection clearance and physically supported ordinary
passages without runtime solving. Fresh copied assemblies load at 0/37/90/180
degrees and two elevations. Native checks pass 120/120 ordinary jump clicks at
0/37/180, plus eight walks and eight supported-floor jump clicks at 90 degrees
(hidden selection on the covered return side). The perpendicular-offset probes
still fall outside the narrow folded projection; they are not reclassified as
passing. Full Nottingham retains its prior 375/390 ordinary-click outcomes.
All 267 selected compiler/export tests pass. The roof metadata is now published.
Eight placement ZIPs reopen and recompile identically; a separate browser bake
preserves actual rendered color/depth through reopening and rebaking. Its four
native click routes pass with all published jump overlays retained. Full-map
rendered input and in-game visual verification remain open. No production
engine changes are involved. See the
[publication and ZIP evidence](map-compilation-evidence.md#published-roof-metadata-and-rendered-zip-round-trip-2026-10-12) and
[compiler evidence](map-compilation-evidence.md#export-time-folded-floor-preparation-2026-10-12).

**Moved-roof takeoffs repaired (2026-10-12):** clearance now uses the precise
exported motion contour used for elevation-boundary preparation. This removes
fictitious ground drops without bypassing collision checks. Eight copied,
rotated and raised fixture ZIPs round-trip exactly; native upright, sword,
approach and player-click checks each pass 120/120 traversals. The 90-degree
layout remains too narrow and correctly emits no jumps. See
[the takeoff evidence](map-compilation-evidence.md#precise-roof-takeoff-boundaries-2026-10-12).

**Jump partitioning (2026-10-12):** export now splits roof-crossing jump spans
and their clickable landing zones by receiving face. Fresh Nottingham passes
367/390 click cases: all 20 cases for the split connection pass, while the
other 370 outcomes were unchanged at that stage. The subsequently published
courtyard-wall ledge inset fixes its two endpoint defects: the upright report
now passes 368/390, with ten expected helper-required posture rejections and
twelve remaining geometric or click-routing failures. Required-posture checks
pass 20/20 in full physical geometry and 160/160 across copied, rotated and
raised wall assemblies. These posture fixtures isolate jump overlays; they do
not certify full-map overlay interactions. See the
[courtyard repair evidence](map-compilation-evidence.md#courtyard-wall-jump-inset-2026-10-12). All 239
compiler/export tests passed at that stage. The subsequent copied-roof failure
is repaired and verified above; full-map jump failures remain under review.
The revised roof-slab candidate fixes its ten full-map samples. The receiving
seam export fix plus asset-local sloped click zones pass all 120 moved/copy
checks at 0/37/180 degrees. The earlier 90-degree export omitted its connection;
the folded-floor preparation above now connects it and passes supported-floor
click routes. The roof metadata has since been published after mesh and ZIP
review; see the [candidate evidence](map-compilation-evidence.md#unpublished-roof-slab-connection-candidate-2026-10-12).
The published southeast-house ground click-band repair removes its click-zone
misses and passes 160/160 moved cases. Four apparent stalls were audit errors
around the native zero-distance stop reservation; runtime behavior is unchanged.
The full-map upright audit now passes 369/390, with ten helper-required
rejections and eleven remaining failures. ZIP/render verification remains open. See the
[ground landing audit](map-compilation-evidence.md#ground-landing-click-band-candidate-2026-10-12).
The combined ground/roof candidate now loads natively and passes 375/390 ordinary
clicks. Its remaining five covered-roof samples pass hidden-selection traversal;
the ten helper-required samples have a passing posture-aware fixture that isolates
their jump overlays. The deeper endpoint-click audit passes 356/390 and retains
34 failures for investigation. The roof assets are now published, with the
additional folded-floor and rendered ZIP evidence above. See the
[combined candidate evidence](map-compilation-evidence.md#combined-nottingham-candidate-and-endpoint-clicks-2026-10-12).
Five of those ordinary-click failures hit an overlapping higher roof. Hidden
selection finds the intended jump in all five cases. Retaining that selection
through command dispatch fixes all five complete walk-jump-walk routes; ordinary
clicks still select the covering roof. Rendered and queued Shift-action checks
remain open. Separate approach checks pass all ten routes across the pair. The broader deeper
approach/endpoint audit passes 336/390, with 54 failures requiring review; see the
[roof overlap evidence](map-compilation-evidence.md#north-stone-house-jump-receiving-overlap-2026-10-12).
See [partition verification](map-compilation-evidence.md#compiled-jump-receiving-face-partitions-2026-10-12).

Fresh Wychford gameplay data loads natively and passes 96 sampled ordinary
routes. Its church-side-tower entrance remains about 46 units above terrain
and is omitted with a warning. The asset expects a raised landing: Leicester's
separate lower-bailey terrace supplies it, while Wychford has no authored asset
floor there. The tower's other three endpoints match its internal floors.
An actual landing/connecting route remains to be authored; changing only the
endpoint would invent a floating connection. See the
[support audit](map-compilation-evidence.md#wychford-tower-entrance-requires-an-authored-landing-2026-10-12).
The baseline ten-map batch finished with nine
successful exports and one Croisement03 fallen-log cap failure. An offline log
repair passes ten placement loads and 17,880 sampled sight/projectile queries;
full-scene re-export then exposes an equivalent timber-bridge cap failure.
The bridge repair passes ten placement loads and 26,800 sampled queries with
its deck and connection definitions unchanged. Both repairs are published;
fresh full Croisement03 now compiles and passes native geometry construction.
Its ordinary-sector route sampler also passes; rendered ZIP and full traversal
verification remain pending. Native batch loading now exposes a Lincoln passage
with no receiving floor (door 47, layer 3, sector 153). Export now catches this
and, in best-effort mode, warns and omits the malformed passage. Fresh Lincoln
from the published library loads natively; restoring that passage remains open.
An asset-local endpoint candidate remains unpublished because collision still
blocks its crossing. Nottingham, Sherwood and York also pass native construction.
Earlier bypassed roof-flight diagnostics are superseded by the production
clearance verification above.

**Boundary repair under validation (2026-10-12):** static export/load corrections
bring the upper-red-house eight-placement jump fixture from 50/80 to 70/80,
resolving all twenty receiver-loss cases. Ten rotated click-selection failures
remain. All 159 compiler tests, 91 level-data tests, 203 movement tests and 72
native integration tests pass, as does the game build. Fresh ten-map checks
are pending. No per-tick geometry solver is added. See
[the boundary evidence](map-compilation-evidence.md#receiver-coverage-at-compiled-floor-boundaries-2026-10-12).

**Current Nottingham jump coverage (2026-10-12):** fresh export
`saved-map-exports-mog5ON` has verified traversal for all **390 sampled cases**:
375 normal clicks, ten assisted ascents and five hidden-floor clicks. The helper
audit now retains all map overlays and passes all twenty ascent/descent cases.
Descriptor hashes, line indices, click parameters and approach depths match
across modes. This closes the isolated-helper-test gap, not all moved-placement
or rendered-actor coverage. See [full-overlay evidence](map-compilation-evidence.md#nottingham-helper-routes-with-all-map-overlays-2026-10-12).

**Current Nottingham ZIP verification (2026-10-12):** the fresh 57,945,741-byte
archive reopens and rebakes identically, including ten appearance states, and
passes native loading without base-game data. Its complete gameplay descriptor
matches the 390-case traversal audit. Native offscreen GPU checks also pass all
1,024 control combinations with transition suppression and reset at six
color/depth probes. Native rendered-character coverage remains open. See
[the archive evidence](map-compilation-evidence.md#full-rendered-nottingham-archive)
and [the GPU and clipping regression checks](map-compilation-evidence.md#nottingham-exhaustive-archive-gpu-check-2026-10-12).

**Earlier Nottingham ZIP verification (2026-10-12):** the full 57,942,452-byte archive reopens
with identical gameplay, color and depth, and loads through native mod/resource
loading without a base data directory. Fresh gameplay data passes 184 ordinary
route samples and retains 355/380 successful jump clicks. The remaining 25
findings are not resolved by the ZIP verification. See
[the fresh Nottingham checks](map-compilation-evidence.md#fresh-nottingham-boundary-fix-regression-checks-2026-10-12).

**Current export repairs (2026-10-12):** four split York assets now have published
gameplay bindings and updated saved-scene pins. Thirty-two moved/rotated exports
load natively, and both storehouses pass eight complete-character enter/exit
round trips. Separate support layouts test door-height terrain and low-ground
masks; the combined scene and rendering remain unverified. Croisement01's
degenerate cap-plane crash is fixed: its fresh export loads natively and passes
16 sampled ordinary walking routes and 130 directly dispatched jump/landing
traversals. A published asset-local stump socket inset now brings the corrected
player-click walk/jump/walk check to 130 of 130 cases. The fresh full-scene export
loads natively and takes about four and a half minutes; this still does not
certify rendering or every possible route.
Croisement02's collapsing caps are traced to mesh-derived wood assets. Published fence repairs
pass 20 moved/rotated native exports and 33,160 mesh-derived sight/projectile
checks, with measured loss below 0.003% of footprint and 0.002% of volume.
Supplemental wood 09/44 repairs are now published too. Croisement02 exports and
passes native loading, six ordinary route samples, 35 unassisted jump-click
attempts and all 40 assisted attempts. The five rejected unassisted attempts
correctly require a helper. With the mask repair below it retains 1,728 non-draft
warnings and existing mesh/collision mismatches. The click-route audit now checks
authorized starting boxes and destinations rather than unchecked synthetic points.
Isolated stump tests pass 622 traversals and identify 18 clicks hidden by the
upper platform; those 18 are not traversal successes. York passes 280 ordinary
route samples, but its click sweep still has 54 failures, including missing
receivers and stalled approaches (655 traversals pass; 11 clicks are occluded).
These results do not establish
full parity; see [the current evidence](map-compilation-evidence.md).
The Croisement01 socket inset is published; four rotated click-overlay misses
remain in its isolated two-asset sweep. A staged York anchor correction improves
the full-scene result to 665 successful traversals and 44 failures, with no
previously passing case regressing. York receiver/rotation repairs remain under
review and are not published.

The subsequent receiver-specific York zone candidate improves this to **680
successful traversals, 11 occluded clicks and 29 failures**. It removes all
missing launch/landing receiver errors, with no previously passing case
regressing. These eight asset definitions are now published with draft warnings;
64 isolated placements load natively, but their internal-pair click tests still
have 48 failures out of 160. Cross-asset placement coverage remains incomplete.
Fresh Derby passes 144 ordinary routes and all 20 required-posture jump cases.
Its browser export exposed unsupported GLB PBR materials in the depth pass;
standard/physical material depth now passes rendered coverage tests. The full
Derby editable-ZIP round trip now passes: reopened gameplay and every rendered
color/depth pixel match, including appearance states. The 25 MB archive also
loads through native mod discovery, image/animation decoding and engine map
construction without a base datadir. This does not certify visual accuracy or
every playable route.

**Nottingham receivers (2026-10-12):** twelve corrected ledge zones across ten
assets are published with draft warnings. Full-scene traversal improves from
280/380 to 355/380, with no passing case regressing; 25 failures remain. All 80
isolated placement exports load natively, but only 522/850 attempted upright
traversals pass (ten additional clicks are occluded). A separate required-helper
fixture passes 50/80 cases and exposes receiver loss during the jump-up landing
animation. This is under investigation; no runtime behavior changed. Fresh
published York also loads and reproduces its 680 traversals, eleven occlusions
and 29 failures. Neither map is at full parity.

**Mask binding update:** coverage-based layer binding restores Croisement02 shrub
77's missing mask. The fresh 53-mask export loads natively and changes only masks
and warnings relative to the previous export. Compiler/packaging tests pass;
rendered occlusion is still unverified.

**Matched runtime comparison (2026-10-11):** current code and the earlier
post-solver-removal baseline pass 9,720 stock frames across H07/H01/S01 using
identical benchmark code, game data and CPU affinity. Both show similarly large
timing variation; these runs do not isolate a consistent current-code regression.
The frame-5 freeze does not recur. A universal 1 ms budget and full rendering
costs remain unverified; detailed measurements are in the runtime inventory.

**Jump audit completeness (2026-10-11):** mismatched opposing edge vectors no
longer suppress the independent flight-clearance audit. Best-effort exports
retain authored connections and report both findings. Fresh hall/walkway exports
now reveal possible flight/takeoff intersections at 0/37/90 degrees and unverified
forward-takeoff geometry at 270 degrees. The earlier successful sampled native
routes do not certify clearance across every edge parameter. These warnings
require further geometry review; no jump endpoints or runtime behavior changed.
See the [jump-audit evidence](map-compilation-evidence.md#jump-vector-warnings-no-longer-hide-clearance-findings-2026-10-11).

The subsequent clearance repair separates vertical flights from long-jump
takeoff constraints. Clear 1/10/15-unit vertical gaps now assemble successfully;
long jumps still require their projected walking takeoff. Vertical direction is
checked in world XY because elevation can project a valid landing behind its
source. Fresh hall/walkway exports now reach clearance checking at all four
rotations. Their gameplay geometry is unchanged and passes 160 native direct
vertical jumps plus 160 click-driven routes. Solid-intersection warnings remain
unresolved, including supporting-wall contacts. No runtime changes were needed.
Contact warnings now identify the placed asset part/volume, directed edge and
affected percentage ranges. Repeated hits are merged per blocker/direction;
copied instances retain distinct identities. Fresh hall/walkway diagnostics
are in `work/needs-review/map-compile/hall-walkway-connection-AmwMS8`, with unchanged gameplay
geometry. These are conservative clearance contacts, not a certification that
the rendered character intersects each named part.

**Rendered reveal round trip (2026-10-11):** two independent hall/walkway pairs
at 37 degrees now pass a 5000×5000 browser export/reopen/rebake check. Both reveal
regions change color and depth independently; all applied-state PNG pixels,
region bounds and control-index bindings survive ZIP packaging and reopening.
The resulting ZIP also passes the native OpenGL appearance gate without a base
datadir: both independent regions switch color/depth through all four control
combinations, transition suppression and reset. Two changing-pixel probes check
30 rendered colors and 60 sprite occlusion results. This assigns control flags
directly; actor activation and full-scene compositing remain unverified.

**Roof jump groups (2026-10-11):** sixteen fresh upright/crouched roof exports
preserve descriptors, warnings, scenes and asset pins through editable ZIP
round trips. Three-PC group clicks pass 48 directed routes / 144 complete sprite
jumps across two copies, rotations 0/37/180 and elevations 0/40. Each actor executes
the long-jump animation and reaches the correct sector, layer, receiver and
required landing posture; launch and final footprints remain separate. The
90-degree narrow connections are still correctly omitted in both batches.
This does not certify in-flight actor separation, assisted group climbs or
rendering. See [group-jump evidence](map-compilation-evidence.md#group-clicks-across-exported-roofs-2026-10-11).

**Cropped spline mask receivers (2026-10-11):** a trimmed point anchor can now
retain its mask when it identifies one asset-owned floor with surviving geometry.
Export derives contact samples from that floor's clipped triangles after bending,
preserving holes and raised receiving layers. External, ambiguous or absent
receivers still warn and require explicit asset-local points/probes. Straight and
curved elevated cases pass editable ZIP round trips and native bitmap, layer,
volume-ownership and altitude checks. No runtime changes were needed; see
[receiver evidence](map-compilation-evidence.md#cropped-point-mask-receivers-2026-10-11).

Rising-floor follow-up: directly warped anchors can miss the triangulated floor
even when they were not cropped. Floor-derived sampling now covers all point-only
masks with an unambiguous asset-owned receiver. A curved span rising 30 units
previously lost two of its three masks; it now retains all three. Four straight/
curved, flat/rising cases pass ZIP reopening and native layer/volume/altitude
checks. Layer numbers are rebuilt and are not assumed to identify ground versus
raised floors. Movement-grid rounding warnings remain separate unresolved findings.

**Raised-floor click ordering (2026-10-11):** normal PC clicks exposed a curved
rising floor ordered below terrain despite valid direct paths. Export now orders
overlapping navigation layers by receiving height before assigning references.
Four editable ZIP round trips, 16 complete-sprite PC walks, 16 direct routes and
88 sampled routes pass; per-tick checks cover receiver, height and collision.
Conflicting height order warns without dropping geometry. Self-overlapping regions,
cyclic ordering and full rendered coverage remain open. No production runtime
changes were needed; see [click-order evidence](map-compilation-evidence.md#click-ordering-on-raised-spline-floors-2026-10-11).
Conflicts are isolated to their connected groups so that unrelated and clearly
higher layers still receive the correct order. Derby's reported conflicts trace
to three small ramp/landing overlaps; their geometry still needs resolution.
Warnings report exported layer IDs and projected overlap locations. Two seams
are internal to the east-bailey curtain assets; the third is the keep/hall bridge
against the upper east curtain. Fractional movement preservation alone does not
resolve the authored overlaps.

**Overlapping joined floors (2026-10-11):** separate regions sharing an initial
layer now receive distinct layers when their placed floors overlap at different
heights. This fixes a new ramp assembly whose raised end otherwise shared the
lower floor's layer. Joined pieces remain one sector, and nonconflicting regions
can still share layers. Assembly and full compiler regressions pass, alongside
four spline ZIP round trips and 16 native spline walks. The dedicated overlapping
assembly additionally passes eight editable ZIP round trips and 64 normal PC
walks across two copies, four rotations and two elevations, checking layer, sector,
height and collision on every tick. Eight additional under-deck ZIP round trips
and 64 lower-floor walks pass with the same layer/sector/receiver checks.
An explicit solid ceiling adds sixteen ZIP round trips: 64 moves pass with
sufficient authored headroom, and 64 low-clearance moves are rejected after at
least 120 observed ticks. Physical ceiling geometry stays identical between the
two policies. Automatic posture selection, self-overlap within one region,
group avoidance and rendered appearance remain open;
see [joined-floor evidence](map-compilation-evidence.md#overlapping-joined-floors-2026-10-11).

Shared layers can also create a false cycle when disjoint lower and raised
regions lie on opposite sides of a middle floor. Export now separates those
groups when their individual regions have a consistent height order. Eight ZIP
round trips and 64 native walks pass across copied/rotated/elevated three-level
assemblies; Derby's real seam warnings remain unchanged. See
[middle-floor evidence](map-compilation-evidence.md#shared-layers-around-a-middle-floor-2026-10-11).

Self-overlap inside one joined region now emits an explicit asset/region warning
and projected location instead of silently treating both heights as one floor.
Geometry is retained for best-effort export; holes are respected. Automatic
partitioning and checked passage generation now handle precise matching-height
seams in compiler/ZIP regressions. All eight editable loop ZIPs round-trip exactly
and all 64 native walks pass, including rotated passage handoffs. Long contour
edges receive additional static graph nodes so the native bounded source search
can enter a route even when its detour corners are far away.
Joined regions now retain placement precision automatically: another eight ZIP
round trips and 64 native walks pass without per-surface precision flags.
Inconsistent authored seams that cannot be reconnected retain warnings; broad
asset coverage remains unverified. See
[automatic precision evidence](map-compilation-evidence.md#joined-floors-preserve-placement-precision-automatically-2026-10-11),
[partitioning evidence](map-compilation-evidence.md#partitioning-joined-floor-loops-2026-10-11)
and
[self-overlap evidence](map-compilation-evidence.md#self-overlapping-navigation-regions-2026-10-11).

A separate projected-geometry climb sweep still fails at 158 degrees for one wall
variant. Real editor exports at 157/158/159 degrees pass 36 prepared native routes;
this narrows the failure but does not certify the synthetic sweep or all angles.
The subsequent full-circle editor audit finds 48 failures among 2,160 directed
routes: all three climb variants stop against projected boundaries at 5–8 and
172–175 degrees. Export-time footprint compensation now passes all 2,160 routes
with no permission skips (`climb-angle-routes-gAccNf`). This covers three variants
at every integer rotation, elevated by 40 units. The eight repaired angles also
pass 144 open/closed/reset checks. The apparent barrier bypass at 283 degrees
was caused by an undersized test grid; all 18 checks there pass with the exported
dimensions. The corrected full-circle barrier audit passes all 6,480 directed
initial/closed/reset checks across 1,080 fixtures. Normal clicks, ZIP round trips
and the synthetic sweep remain unverified or unresolved. See
[full-circle evidence](map-compilation-evidence.md#full-circle-editor-climb-audit-2026-10-12-incomplete).

**Authored occupancy triggers (2026-10-11):** eight new map exports place two
independent reusable trigger assets at 0/37/90/180 degrees and elevations 0/40.
All ZIP extraction/editor reopen/recompile checks pass. Normal native player
movement passes 64 enter/leave walks: two PCs are tracked independently, the
state stays applied until the last exit, and initial/applied sight obstacles
switch without affecting the other copy. Positions retain their authored height.
Another 64 walks pass with one PC carrying a soldier: carrier/body membership
enters and leaves together, carrying posture is preserved, and the remaining PC
keeps the state applied. Trigger polygons come from asset metadata; actors exist
only in the native harness. This does not add triggers to assets with empty
polygons, invent mission scripts, or certify independently moving NPCs, pickup/drop
inside the trigger, or rendered activation. See the
[occupancy-trigger evidence](map-compilation-evidence.md#authored-occupancy-trigger-round-trips-2026-10-11).

**Exported guard visibility (2026-10-11):** eight editable ZIP round trips with
two independent tall sight barriers pass 320 native visibility checks. Normal
NPC visibility and 180-degree detection both follow initial/applied obstacle
positions through separate apply/reset operations at 0/37/90/180 degrees and
elevations 0/40. Actors are supplied by the test, not inserted into map exports.
This verifies live perception queries; patrol acquisition, alarm propagation and
rendered encounters remain separate checks. No production runtime changes were
needed. See the [sight-barrier evidence](map-compilation-evidence.md#exported-sight-barriers-and-live-guard-detection-2026-10-11).

**Hall/walkway dependency (2026-10-11):** the hall's reveal waypoint belongs on
the separate east-bailey west-curtain walkway. A flat-ground-only scene cannot
bind it correctly. Joining those reusable assets restores the control without
changing metadata; moving the walkway away removes it with explicit warnings.
Two copies at 0/37/90/270 degrees retain independent appearance and sight-state
references. Native apply/reset checks pass for all eight retained reveal controls,
including whole-world sight-state isolation and grid/navigation restoration.
Joined exports have 16–18 warnings versus 108–110 when separated.
The isolated hall's missing reveal is therefore a missing neighboring surface,
not proof of a bad asset-local anchor. Jump-edge quantization and other draft
warnings remain unresolved.

Native jumps in those four joined scenes now pass 160/160 direct sprite-dispatch
cases, including assisted climbs and upright descents, and 160/160 click-driven
routes. Two former failures at 270 degrees came from an acute stair-clearance
corner extending over a thousand units beyond its floor and intercepting clicks.
Export now bounds that clearance by the projected floor plus the actor footprint;
no runtime behavior changed. Fresh ZIP round trips at 270 degrees and elevations
0/40 also pass all 32 stair routes in each of five native routing modes.
The opposing jump-edge warning remains, but does not itself prove failed traversal.
The broader matrix also exposed a pre-existing receiver-binding mismatch at
90 degrees/elevation 40 without map padding. Export was unnecessarily splitting
one stair receiver when native XY vertices coincided but their heights differed
slightly. Deduplicating that XY outline while retaining the separate height plane
fixes the placement: fresh ZIPs pass all 32 routes in each of five native modes.
The final unpadded eight-angle matrix passes 256/256 prepared, sprite, discovered
and hidden-selection routes. Ordinary clicks pass 252/256, with the four covered
217-degree targets requiring the explicit hidden-selection modifier.

**Browser bake/reopen round trip (2026-10-10):** two independently placed
east-hall assets on elevated editor terrain bake to a real 2000×2000 color/depth
export. ZIP decoding and reopening/rebaking through the browser viewport preserve
the pixels and gameplay descriptor exactly. A second scene adds one PC spawn
and one soldier: both survive with correct elevated receiver bindings, while map
geometry and PNGs remain identical. The browser-produced map-only descriptor
passes **16/16 stair routes in all five native modes**.

This is not full asset parity: the export reports **150 warnings**, including
omitted `hall-reveal` controls, unbound entrances and appearances held in their
initial state. Some depend on neighboring surfaces, as verified above; the
remaining omissions still need assembly-specific diagnosis. The
explicit frame also clips the first tower's top. Browser component integration
is covered here; full editor button flows, game-menu installation and in-game
image/actor compositing are still open.

**Packaged stair round trip (2026-10-10):** the placement verifier's
`--zip-roundtrip` mode packages the export, reads its descriptor back from the
ZIP, reopens the pinned editor document and recompiles it. Descriptors and
warnings match. Two east-hall copies at 270 degrees and terrain elevations 0/40
retain their precise stair contours and pass **32/32 routes in each of five
native modes** using the ZIP-extracted descriptors. The diagnostic ZIP images
are blank; this does not certify rendering, installation through the game menu,
or browser editor reopening.

**East-hall eight-angle traversal (2026-10-10):** removing a redundant,
almost-collinear clipping vertex restores clearance preparation at 217 degrees.
Fresh two-copy exports at eight angles and elevations 0/40 now pass **256/256**
prepared, sprite, gate-discovered and hidden-selection routes. Ordinary clicks
pass **252/256**; four covered targets require hidden selection. This verifies
the tested stair placements, not complete map gameplay or rendering parity.

**Subpixel stair clearance (2026-10-10):** the exporter preserves precise
clearance contours, constructs graphs against them and chooses supported integer
endpoints. Startup binds those contours into existing static collision and sector
data. At this stage east hall at 270 degrees passed **32/32 in all five modes**;
the remaining 217-degree failure was subsequently fixed by the clipping cleanup
above. This introduces no per-tick solver.
The stock regression completes all 540 frames, but its latest timing repeat
exceeds the historical baseline. Approximately 1 ms ticks are not currently
certified; see the runtime inventory for measurements and the unresolved gap.

**Earlier east-hall audit (2026-10-10):** all five modes pass 24/32 cases after
correcting the expectation that movement between doors on one landing must detour
through a stair. Gate/player audits separately report eight same-landing walks;
prepared audits still force the stair traversal. The remaining eight failures
in each mode concern the lower stair. Upper-west curtain remains 64/64 in all
five modes. This is an audit correction, not additional gameplay parity.

**East-hall receivers (2026-10-10):** receiver boundaries now retain the exact
placed floor instead of reconstructing heights from rounded movement geometry.
The nearly edge-on lower stair now exports its actual 0–110 height range. Route
counts remain unchanged: its rounded clearance polygon still cannot fit the
walking footprint, and same-landing route expectations need a separate audit.
No runtime solver was introduced.

**East-hall placement remains incomplete (2026-10-10):** best-effort export now
survives a floor fragment collapsing during 270-degree placement by deferring its
rounding until floor union. Final unrepresentable regions are warned and omitted;
strict validation and collision blockers retain their existing behavior. Fresh
two-copy exports at elevations 0/40 pass only 24/32 prepared stair traversals and
16/32 discovered/player-click routes. Receiving-obstacle mismatches and interrupted
movement remain to be fixed in export preparation; this is not a parity pass.

**Fresh upper-west curtain placement (2026-10-10):** Derby's published asset passes
all 64 directed routes in each of five current-engine audit modes: synthetic and
complete-sprite traversal, gate discovery, ordinary clicks and hidden-selection
capable clicks. None requires hidden selection. The sixteen newly inserted,
saved/reopened exports cover two copies, eight rotations and two elevations.
Rendered integration and broader actor/state coverage remain open. The local
verifier now streams catalog entries after the 898 MB index exceeded Node's
single-string limit; this does not certify large-catalog editor loading.

**Fresh stair-house placement (2026-10-09):** Nottingham's south stair house now
has current prepared-graph coverage on new editor terrain: two independent copies,
eight rotations and two elevations retain every lift and entrance. All 64 directed
routes pass synthetic traversal, complete-sprite traversal and gate discovery.
Normal non-jumping-PC clicks pass 60/64 routes, including sixteen clicks on jump
overlays that select their underlying floor. The remaining four upper landings
are covered by the roof at 217 degrees and have no ordinary selectable floor
target. A separate hidden-sector selection audit now passes all 64 routes, using
the existing hidden lookup for exactly those four cases and preserving its chosen
sector/layer through normal movement dispatch. Ordinary-click coverage remains
60/64; no cases are skipped. Host modifier events, planning-queue playback,
rendering and the asset's existing draft warnings remain open.

**Fresh keep mask placement (2026-10-09):** the great keep's west-view mask now
has a finite asset-local ground receiver probe. Two independent copies at four
rotations and two terrain elevations retain all twelve masks, versus ten before
the repair. The five raised masks per copy and all other gameplay remain
unchanged, including correctly remapped mask-control indices. Saved Leicester
and Wychford compile and load native geometry without keep-mask warnings.
This verifies receiver binding and data preservation; rendered compositing
remains open.

**Fresh stair placement regression (2026-10-09):**
`refinement/check-grounded-stair-placements.mjs` inserts the reusable church-side
tower through the editor's asset insertion API onto new flat terrain, serializes
and reopens each document, then exports two independent copies at eight rotations
and two elevations. Grounding retains both stairs and all four entrances per
copy. The expanded complete-sprite native stair audit now passes all 128 directed
routes at 0/37/90/137/180/217/270/317 degrees, including completed-passage receiver
checks. Its 500-pixel padding keeps asset receivers inside the export frame. No
routes are skipped as forbidden. Stair passage preparation can move a blocked sector handoff toward
the landing along its existing approach, bounded by one native actor radius.
Both animation segments must clear their floors' permanent obstacles; endpoints
and collision geometry stay unchanged, and state conditions are compiled again.
This restores eight 0-degree routes without disabling their collision checks.
The eight lower-stair stalls at 180 degrees are fixed at export: when a convex,
unobstructed floor projects too narrowly to fit a native actor, the compiler
adjusts its collision boundaries to preserve physically supported movement
footprints after projection. Quantization includes an inward safety margin.
Already traversable floors retain their boundaries and prepared entrances;
ladders, walls, holes, obstacles, bent and singular floors are excluded from this
correction. Physical receiving surfaces remain unchanged. Placement checks pass
at both elevations and for both independent copies. The upper entrance's
footprint overlaps the landing's stairwell hole before handoff preparation.
The 90-degree corridor has no full-box center region. The exporter now recovers
a conservative inner kernel only when the original concave projection has no
space for a full actor box. On an unobstructed convex stair, it can prepare all
inner endpoints together, preferring full-box clearance when available and
otherwise using the native direct-movement inset. Every endpoint pair is checked,
including the extra left margin on horizontal sweeps. Approaches may turn toward
valid space instead of extending beyond the stair. Valid full-box approaches,
obstacles, holes and incomplete endpoint recovery remain unchanged. Receiving-boundary construction
at load time now binds the handoff to the first receiver reached along its
approach, preserving subsequent transitions between receiver pieces.
Grounded copies now retain their projectile masks: the published asset contains
a finite receiver probe between its old local mask anchor and its authored
insertion ground height. All sixteen padded exports retain two terrain-bound masks with
independent obstacle ownership and load natively. Saved Leicester also exports
without a tower-mask warning; saved Wychford retains its previous missing-receiver
warning for the existing placement. No receiver outside the finite probe is
invented. The additional four angles also pass 64 routes at the original
coordinates. At those coordinates, 270-degree mask probes reach the frame edge
or lie outside it; omitted masks remain explicit crop warnings. Cropped receiving
probes remain a separate limit.
The native audit enforces the
manifest's expected directed-route count and reports incomplete coverage when
any required route is forbidden. A separate native gate-discovery audit also
passes all 128 routes, selecting the intended entrance/exit on the intended copy
and executing the returned path. A normal single-PC player-click audit also
passes all 128 routes with no sector or route overrides. Its targets are the
nearest visible, actor-sized landing points with clear endpoint approaches:
24 covered endpoints need a nearby click, at most 8.485282 units away. Completed
zero-travel stop animations allow only their native 0.01-unit residual plus
coordinate rounding; passage and receiver checks remain unchanged. AI decisions,
multi-PC formations, further angles and broader actor/posture interactions remain
separate gates.
No runtime routing solver is required
or reintroduced by this audit.

**Broader verification (2026-10-09):** the full editor suite now reports 1,069
passed, two skipped and one failure: the named-size preset test reads a locally
edited Croisement02 scene with `size: null`. That scene was left untouched.
The export-related failures were resolved: the final jump audit now retains
placed receiver identities and uses the opposite line's destination reference
to identify each source zone. Rotated roof fixtures match again without changing
their expected output. Six terrain/navigation fixtures were refreshed to include
explicit receiving-plane anchors and current native contour ordering; their
navigation graphs are unchanged. Native map integration passes 71 tests (five
ignored), and actor navigation passes 26 (32 opt-in audits ignored).

Fresh saved-document exports after the clearance correction load natively:
Wychford constructs 86 areas, 31 doors and no jump pairs; Derby constructs 60
areas, 70 doors and two jump pairs. Ordinary initial-state route sampling passes
98 Wychford routes and 144 Derby routes. Wychford takes about 467 seconds to
compile, including best-effort retry; Derby takes about 2.7 seconds in the same
run. These are geometry exports, not browser ZIP/rendering certification.
Wychford still omits the church-side-tower traversal because its placed endpoint
does not meet the receiving surface, and both maps retain explicit geometry
warnings. Successful loading and sampled routes do not certify those omissions.

**Takeoff clearance progress (2026-10-09):** long-jump clearance covers sprite
takeoff positions between the ledge and the nominal fifteen-unit target, including
receiver changes. The compiler catches stationary upright and intermediate sword
departures; recovered roof and rock connection regressions pass. Leaving a
receiver polygon does not necessarily drop takeoff to ground. Native movement
retains the bound plane until an elevation boundary is
crossed; boundary generation is clipped to motion areas. Export-time takeoff now
sweeps these boundaries, with tests for retained slopes and subpixel side probes.
The latest boundary calculation passes 120 full-house sword cases and all 2,920
recorded positions, including rotated takeoffs that defeated polygon-only lookup.
Increasing every ledge inset would
remove usable landing regions on narrow rotated roofs and is not a validated fix.
The current model covers 16,130 recorded native positions across 480 upright,
sword and shoulder-assisted cases. Takeoff receiver selection now follows adjacent
planes in the same sector/layer and chooses overlapping receivers by their top
bound. The latest selected jump/compiler/export run passes all 263 tests.
Obstacle-trimmed endpoints were refreshed after 60 native flight-clearance checks
and 20 assisted traversals passed; their 770 recorded positions are covered by
the exporter. These small fixtures do not provide sufficient approach space for
the general click-route audit, so that audit does not certify them.
Assisted departures now use the bounded flight calculation with an in-place
forty-unit lift; native trace coverage passes 120 full-house cases (3,580 positions)
and 160 synthetic cases (5,840 positions). Fresh roof exports now pass 720 native
traversal cases across six audit modes, retaining connections at 0, 37 and 180
degrees at both tested elevations. The recovered-roof regression passes again.
Final landing checks model the atomic receiver binding instead of sweeping a
fictitious intermediate segment. These fresh exports also pass all 120 sword
cases and 2,920 recorded positions, plus 120 assisted cases and 3,580 positions,
through the export-time clearance checker.
Grid snapping can expose new obstruction after a span is shortened; connection
assembly now retries trimming while the span strictly decreases, preserving
usable connections instead of immediately discarding them.
An unresolved shortened span now emits a specific warning without aborting
other spans or independent copied connections. Landing clearance uses the
destination line's midpoint receiver across its whole span, matching runtime
binding even when an endpoint lies outside that receiver polygon. A regression
verifies that an obstacle at the rebound endpoint is no longer missed. Fresh
exports still pass all 720 native roof traversal cases after this correction.
Using the exported jump zones' exact sector/layer also passes the full-house sword
trajectory audit: 120 cases and 2,920 recorded positions. The compiler resolves
these references from placed landing anchors; no saved level navigation is used.
These checks certify the tested placements, not full gameplay parity. Broader
authoring/traversal coverage and the remaining categories below still require
validation; no runtime solver is being added.

**Navigation architecture correction (2026-10-08):** the graph-less runtime
visibility fallback has been removed, with no compatibility replacement. Export
now compiles native `graph_bytes` from placed motion geometry, with actor clearance
and state-dependent nodes/links. No source graph is copied. The loader accepts
this prepared stream through the existing graph parser. The separate physical-floor
solver, load-time bindings, movement
dispatch and per-step routing have also been removed. An optimized three-player
stock-mission regression completes all 540 frames, including Silver Arrow frame 5;
median ticks range from 0.32 to 1.66 ms in the latest repeat, including the
sound-fragment and carrier-recovery changes. This confirms freeze removal, not a
consistent 1 ms budget. Current validation: 248 selected compiler/export/sound tests pass;
native map integration is 71 passed, 5 ignored; the
engine suite is **4,347 passed, 0 failed, 39 ignored**. Passage approaches are now
prepared at export, and the loader's approach search has been deleted. Exports
omit obsolete physical navigation metadata. The four stair receiver/barrier
failures now pass using the native endpoints. Derby's saved-scene export
loads natively and passes 144 sampled ordinary routes (0.01 seconds pathfinding
in the unoptimized harness). This does not certify all scene connections. See the
[runtime inventory](map-runtime-inventory.md) for the full scope and correction
requirements. Earlier physical traversal evidence does not certify this architecture.

**Current roof-jump evidence:** removing consecutive vertices that collapse to
the same native coordinates avoids unnecessary receiver splits during export.
Moved/copied roof assemblies at three usable rotations and two elevations now
pass 120 isolated sprite jumps and landing-receiver checks. The fourth rotation
omits its connections because no character-sized receiving span is generated.
All 120 authored four-unit approaches pass native footprint checks and complete
walk–jump–walk execution. After aligning generated ledges with the graph's 6×3
footprint, all 120 four-unit approaches finish their finite startup
animation 0.8 or 1.6 units short while turning, matching the reference behavior.
The audit verifies each step's distance and animation/command completion rather
than accepting an arbitrary near-goal offset. No runtime change was needed.
Another 120 isolated sword jumps pass exact landing and receiver checks, exercise
the sword-flight animation and preserve combat state and opponent relationships.
Another 120 interior-click cases select the expected connection and complete
normal player movement with exact arrival. The checked-in fixture also verifies
that removing jump skill rejects both directions of both copied connections.
Another 120 shoulder jumps and 120 assisted player-click routes pass with complete
rider/helper animations and carrier recovery. This exposed a missing helper
recovery completion event, now restored without navigation changes. Full combat
interaction, multi-PC click formations and
rendering remain open; see [assisted-jump evidence](map-compilation-evidence.md#assisted-roof-jumps-and-carrier-recovery-2026-10-08).
Automatic cross-asset clearance now supports climbing edges on horizontal receivers,
including native up/down flight, assisted takeoff and the landing lift. Repositioned
neighbours at four rotations and two elevations pass 160 isolated traversals and
160 click routes across independent copies; all 7,840 recorded positions are covered
by export-time clearance. Another 32 out-of-range/excess-rise checks disconnect
only the changed copy. Rigid rotation alone can remove projected overlap between
different-height ledges; those placements remain explicit rejection cases.
Surface rules now support `long: false` and `helperNeeded: true`, deriving all
climbing edges/zones without authored jump records. A separate surface-only batch
passes another 160 isolated traversals, 160 clicks, 7,840 clearance samples and
32 disconnected-copy checks. Missing climbing neighbours warn. Existing surface
rules retain long-flight defaults. Sloped receivers now pass another 960 native
traversal/click checks with all 23,540 recorded positions covered by export-time
clearance, including surface-derived connections. Sloped climbs in the mixed
60–100-unit posture-dependent range now pass another 1,920 native traversal/click
checks with all 38,240 recorded positions covered. Ambiguous receivers, ledges
crossing receiving planes and uncertain zero-length airborne orders still warn
and omit their connections. See
[mixed-posture slope evidence](map-compilation-evidence.md#mixed-posture-sloped-climbing-clearance-2026-10-09),
[sloped-climb evidence](map-compilation-evidence.md#sloped-climbing-clearance-2026-10-08) and
[surface-climb evidence](map-compilation-evidence.md#surface-derived-climbing-connections-2026-10-08).
Flat-receiver rises of 60/80/99 units additionally pass 1,280 native traversal/click
checks, including surface-generated connections. The audit verifies assisted long
flight versus upright vertical descent and covers 25,460 recorded positions with
export-time clearance. See [threshold evidence](map-compilation-evidence.md#posture-dependent-climbing-threshold-checks-2026-10-09).
See [placement evidence](map-compilation-evidence.md#repositioned-automatic-vertical-connections-2026-10-08).
Authored paired connections pass 160 isolated
vertical traversals and 140 player-click routes across four rotations, two elevations
and independent copies, including helper-required ascent and upright descent.
Another 20 cases verify that an upper platform correctly occludes the lower click
target; these are not counted as successful routes. Editor reopen/recompile equality
passes for all eight placements. No production runtime changes were needed. See
[vertical-jump evidence](map-compilation-evidence.md#vertical-jump-export-investigation-2026-10-08).
The 246 focused editor/compiler tests pass. Ledges and graph preparation now use
one footprint definition; an eight-unit-deep roof retains its usable ledge while
six units still fails the rounding margin. See [current evidence](map-compilation-evidence.md#shared-native-footprint-for-jump-ledges-2026-10-08)
and [reproduction and scope](map-compilation-evidence.md#prepared-roof-jumps-and-continuous-receivers-2026-10-08).

**Large-map progress and remaining gap:** Wychford now exports its graph using
[32-bit link indices](navigation-graph-format.md), with no runtime search change.
Graph preparation takes roughly 21 seconds; the complete saved-scene compile
took 257 seconds. Native construction and **98 ordinary sampled routes** pass
(0.14 seconds in pathfinding in the unoptimized harness). A narrow-contour contact
was reproduced in actor ticks and corrected by checking the exact emitted segment
when smoothing a route; shortened endpoints could incorrectly clear a corner.
The actor now completes that route both ways with per-tick collision and height
checks. This is focused navigation evidence, not full Wychford gameplay certification.
Current graph evidence: `work/needs-review/map-compile/wychford-graph-final-iaAtSp`, regenerated
from the complete scene export `work/needs-review/map-compile/saved-map-exports-f0f5B4`.
This supersedes earlier Wychford navigation claims below.

**Full parity is not yet verified.** “Tested” below describes specific evidence,
not certification of every library asset or gameplay case. Best-effort exports
warn about omissions and preserve initial states where possible; warnings are
not evidence that an omitted feature works.

**Partial coverage — forced crouching:** asset-local walkable surfaces accept
`forceCrouched`, which exports the existing native motion-area flag. Ordinary
areas retain separate posture settings; uniformly crouched lift assemblies keep
the flag. Conflicting lift segments fail strict compilation, or warn and omit
the invalid lift in best-effort exports. Explicit navigation regions with mixed
posture remain separate; their boundary needs checked socket passages or
authored passages.
A compiler-generated two-room fixture passes a native waiting-transition test:
the crouch room generates the crouching transition while its neighbour stays
upright. Compiler tests also cover independently copied room passages at
0/37/90/180 degrees and crouched curved rising spline floors above upright ground.
The roof placement audit accepts `--crouched`; the eight resulting descriptors
match the upright control exactly apart from area flags. Native sprite landing,
walk-jump-walk and player-click routing each pass 120 cases across two copies,
0/37/180-degree rotations and 0/40 height offsets, checking the destination's
crouched posture. Evidence: `work/needs-review/map-compile/prepared-crouched-roof-placements-8A1PvE`;
upright control: `work/needs-review/map-compile/prepared-roof-placements-ntgG7I`.
Both variants omit the 90-degree jump pairs because no character-sized receiving
span remains; those omitted connections are not covered by the passing audit.
The narrow roof's projected width is at most 7.21 along the level ledge normal,
while the native axis-aligned character footprint requires 10.38 in that
direction, before rounding margin. A ledge-generation regression checks this
geometric rejection. Rotation can make a steep roof unusably narrow in the
native projected navigation representation; this case must keep its warning,
not export an unwalkable jump connection.
The synthetic copied curved rising deck audit also accepts `--crouched` with
`--deck`. With `--curved --deck --copied --crouched --rotated --zip-roundtrip`,
all 16 diagnostic ZIPs reopen and recompile identically at 0/37/90/180 degrees
and deck elevations 40/80. Native construction passes 384 collision probes and
192 bounded routes, retaining independent deck sectors and upright ground.
Evidence: `work/needs-review/map-compile/spline-clearance-planes-B3ktkm`. These ZIPs use blank
diagnostic rasters, so this does not certify rendered appearance. The first
zero-rotation batch (`work/needs-review/map-compile/spline-clearance-planes-FTFRiO`) passes
48 complete sprite crossings and subsequent crouched waiting transitions,
checking collision, receiving floor, height and no movement while crouching.
The corresponding upright control (`work/needs-review/map-compile/spline-clearance-planes-vq1qHO`)
passes all 48 crossings and remains upright when waiting under the same audit.
The full rotated batch passes all 192 complete sprite crossings and crouched
waiting transitions under the same collision, receiver, height and stationary
posture checks (maximum travelled distance 39.990029 for a 40-unit target).
Explicitly matched sockets between uniformly upright and uniformly crouched
asset regions now emit ordinary non-clickable passages. Compilation checks
integer-rounded approaches and crossings against final floor contours and
permanent blockers, retains one passage per separate clear opening, and warns
when no usable opening remains. Compiler tests cover rotated copies, detached
sockets, narrow openings and blocked/split seams. A compiler-generated raised
two-floor fixture passes native player movement in both directions, retaining
the expected sector, floor height and destination posture through completion.
The expanded `check-posture-passages.mjs` audit passes 24 editable ZIP round
trips: two independent copies at 0/37/90/180 degrees, elevations 40/80 and clear,
split or fully blocked seams. Native player routes from the extracted descriptors
pass all 96 directed crossings, checking completion, corridor adherence, sector,
height and destination posture; eight blocked cases retain zero doors. Evidence:
`work/needs-review/map-compile/posture-passages-DlAdIG/actor-posture-report.json`. Blank diagnostic
rasters do not verify rendered appearance. A rotated opening clipped by an earlier
test frame was correctly omitted; the final audit asserts that no case is clipped.
Ordinary standalone doors now export obstacle-state requirements for both
animation legs, using each receiving area's independent state word. Permanent
cuts are retained for passage checks even when subtraction merges an obstacle
into a floor boundary; authored solid-volume clearances are respected. A blocked
passage stays unavailable and produces an export warning without overwriting
script activation or lock settings. Compiler/export tests pin editable ZIP
descriptors to native fixtures. Native loading and authorization tests verify
permanent blocking, independent copied patch controls, reopening, and preservation
of explicit deactivation. These checks do not yet certify interruption/resumption
of ordinary animated crossings.
Initially closed generated posture sockets now retain their connections:
permanent collision is checked before binding, while temporary obstacle coverage
is represented by exported passage-state requirements. Previously, binding
against the initial obstacle state discarded a connection needed after opening.
Compiler checks cover independent copies at four rotations, and the ZIP
descriptor matches `asset-closed-posture.level.json`. Native checks verify both
connections start blocked, opening/resetting one control affects only its copy,
and all four directed full-character player routes complete after opening with
the correct posture, height and sector. If the control is unavailable,
best-effort compilation retains its initial solid barrier and omits the passage.
The `check-posture-passages.mjs --initially-closed` audit also passes eight
editable ZIP round trips at 0/37/90/180 degrees and floor elevations 40/80.
Native player movement passes all 32 directed crossings after opening the
relevant asset control, checking independent copied availability, corridor
adherence, final posture, height and sector. Evidence:
`work/needs-review/map-compile/closed-posture-passages-OuSqkE/actor-posture-report.json`.
Solid-barrier closure during these posture-changing crossings passes 32 native
cases across the same eight ZIPs, both copies and both directions. The audit
opens the reversible control, starts a full-sprite player crossing, then operates
the control again after movement begins. It verifies that the restored solid
barrier overlaps the actor, blocks the passage and marks the actor unreachable
for crushing without changing the other copy's passage availability. A forced
patch reset is deliberately not used: it restores state without actor
invalidation/damage. Evidence:
`work/needs-review/map-compile/closed-posture-passages-OuSqkE/actor-posture-crushing-report.json`;
reproduce with `exported_posture_barriers_crush_overlapping_crossings` and the
same diagnostics/sprite environment as the crossing audit. This checks damage
dispatch eligibility, not the death animation or final health. Non-overlapping
closure and safe pause/resume during these posture crossings remain open.
Generated posture passages also retain separate openings when changing
obstacles give different parts of the same boundary different availability.
Previously, choosing one static midpoint could leave the connection blocked in
both states even though each state had an unobstructed opening elsewhere.
Compilation now partitions candidate crossings by their receiving-area state
requirements and excludes candidates that can never be clear. Compiler checks
cover rotated/raised copies; the packaged descriptor matches
`asset-alternating-posture.level.json`. Native loading/apply/reset checks verify
each copy keeps one usable opening in either state and its control exchanges
only that copy's openings. The `--alternating` ZIP audit now passes eight
reopen/recompile round trips at four rotations and floor elevations 40/80.
Native player routes pass all 64 directed full-character crossings, including
the initially usable and switched openings, with the expected corridor,
destination posture, height and sector. Changing one copy leaves the other
copy's availability unchanged. Evidence:
`work/needs-review/map-compile/alternating-posture-passages-XgD0Q0/actor-posture-report.json`.
The same eight ZIP-extracted descriptors also pass 64 three-PC group clicks
(192 actor crossings) through both copies, in both directions and both obstacle
states. Normal player dispatch selects the routes and authorizes formation
slots. The audit checks valid starting footprints, settled arrival within 25
units of the click, destination sector/posture/height, and clear, non-overlapping
final footprints. Exact formation offsets are not required because obstacle
authorization can move individual slots. This does not certify collision between
actors during traversal, mid-crossing state changes, or rendering. Reproduce
with `exported_posture_openings_support_player_formations`, using the same
diagnostics directory and character sprite as the single-PC audit; evidence:
`work/needs-review/map-compile/alternating-posture-passages-XgD0Q0/actor-formation-report.json`.
Virtual-room entrances now prepare their outdoor leg against the outdoor area's
state word and retained permanent cuts; the virtual interior has no motion-area
state word. Compiler and ZIP checks cover permanent blocking and warning output,
and native loading/apply/reset checks cover two copied buildings with two
entrances each: blocking one entrance preserves the other entrance, the other
copy, and authored locks. Compiler checks also preserve those independent
requirements at 0/37/90/180 degrees and elevations 0/40. Native full-sprite tests
now pass two unobstructed building round trips and twelve mid-crossing closure
cases across both copies and both traversal directions. Four cases remain
blocked; eight reopen after 20/120 ticks and complete the round trip. While
blocked, map/world position, layer, sector, receiver and animation cursor stay
fixed, and the other copy remains available. The test uses the compiler-pinned
`asset-interior-passage-states.level.json` fixture and runs explicitly with
`ROBIN_CLIMB_RHS` as
`exported_building_entrances_pause_and_resume_complete_actor_crossings`.
The `check-building-passages.mjs` audit additionally passes eight editable ZIP
round trips at 0/37/90/180 degrees and elevations 0/40, each containing two
independent building copies. Native full-sprite prepared routes from the extracted
descriptors pass 16 control round trips and 96 closure cases: 64 reopen and
finish, while 32 remain blocked. Every closure during entry explicitly verifies
that the actor's footprint remains authorized. An earlier closer barrier at
180 degrees overlapped the actor after its first animation step; a separate
native regression verifies the resulting unreachable flag. The safe-closure
audit moves that barrier four asset-local units along the approach. Evidence:
`work/needs-review/map-compile/building-passages-5BWPJQ/actor-building-report.json`.
Blank diagnostic rasters do not certify rendered appearance. A separate native
player-click audit now passes 48 cases from the same eight ZIP descriptors:
screen selection identifies the intended door in each copy; 16 blocked clicks
leave the actor outside, while 32 initially open or reopened clicks leave it
settled in the correct virtual room and layer. No gate path or selected-sector
override is supplied. Evidence:
`work/needs-review/map-compile/building-passages-5BWPJQ/actor-building-click-report.json`.
The same audit now also passes 32 outdoor player-click exits, including 16 after
reopening. Sixteen blocked-exit checks keep the actor in its virtual room for
600 ticks; reopening then completes exit with the correct position, layer,
sector and receiver. Screen hit-testing selects the outdoor landing without a
goal override. The audit applies before resetting a patch and verifies its live
passage state, since the reset operation exchanges the movement-state pair.
The audit's `--trap` variant exports building-trap entrances from asset metadata
and passes the same eight ZIP/reopen round trips, 16 native control round trips,
96 mid-crossing closure cases, 48 player entry checks, 32 exits and 16 blocked
exits. Full character animations exercise the native trap endpoint adaptation;
no runtime changes are required. Evidence:
`work/needs-review/map-compile/building-trap-passages-ZkktWx/actor-building-report.json` and
`work/needs-review/map-compile/building-trap-passages-ZkktWx/actor-building-click-report.json`.
Multiple actors, NPC trap restrictions and broader lock/permission
interactions still need coverage. Changing mixed-posture passages
and regions without explicit sockets still need broader coverage. This is not full crouch-area
parity. No runtime solver was added.

Spline-wall trimming now preserves disconnected closed mask contours as separate
applications, including sloped and vertical boundaries. Moved, bent and rising
compiler checks match separately authored islands; native character/projectile
queries and an editor export/reopen round trip pass. This is application-rule
evidence, not rendered mask certification.
Cropping also preserves height bends at vertices that appear collinear in 2D.
The final Wychford descriptor remains identical to its native-tested publication;
the synthetic browser bake passes PNG and ZIP round-tripping.
The freshly baked ZIP also passes native mounting, image decoding and live
navigation/sight construction without a base datadir.

Export reports gameplay phases from its worker while keeping compilation pending
and cancellable. The saved-map audit records per-phase durations, including
best-effort retries. Eleven focused wall/worker checks pass; phase reporting does
not itself certify gameplay or browser interaction.
Receiving construction also reports landing setup, each region's boundary and
obstacles, and material batches of 64 pieces. Nine wall export checks retain the
same native fixtures; typecheck, lint, formatting and the production build pass.
This instrumentation narrows the remaining slow Wychford export; it is not
evidence of a full-map speed improvement.
The slow first Wychford boundary is now isolated to a union of 850 floor pieces.
Batching their fixed-point union completes that boundary in about 3.1 seconds;
a 504-triangle regression preserves the fractional perimeter and interior gap.
Near-coincident obstacle reconstruction also retries a failing floating-point
intersection at the source collision precision. Coverage and rounding checks
pass for the captured failure. Wychford descriptor export now completes in
126.2 seconds including one best-effort retry (`saved-map-exports-e13P3h`). It
retains 1,862 warnings, including the floating church approach and watermill
receiver omission. The evidence document records the small union-rounding
differences. Native construction and all three controls pass; browser baking
remains a separate check.
Native floor binding repeats a costly boundary-validity test for each receiving
plane. Pruning disjoint edge pairs before the unchanged intersection predicate
reduces a four-case loading benchmark from 79.96 to 8.12 seconds. All 44
stair-navigation, 55 enabled compiled-navigation and 67 enabled map-compilation
tests pass. The same full Wychford export passes construction in 255.00 seconds
and all three control apply/reset checks in 260.72 seconds, down from roughly
13 minutes each. These are single-run debug audit timings.
The published watermill draft supplies its own raised platform floor,
retains both material regions, seats the upper doorway on that floor, and derives
a shorter inset jump span. Across copied, rotated and elevated placements it
passes 160 upright walk–jump–walk routes, 160 isolated sprite jumps and all 64
endpoint-zone checks. The doorway correction passed 48 complete-sprite round
trips. Eight lower-terrain negative cases omit unsupported lower entrances and
jumps. A complete 480-case flight audit finds 80 uphill sword-fighting flights
intersecting the platform; 160 upright, 160 assisted trajectories and 80 downhill
sword flights clear solids. Assisted trajectory checks do not exercise complete
sprite dispatch. In addition,
3.431 square units of the floor footprint lie outside the sampled top mesh.
Combat/assisted sprite dispatch, click authorization and rendered contact remain
unverified. The library retains these explicit best-effort draft warnings;
full parity is not claimed. All ten saved scenes reopen with refreshed pins.
Fifteen inset/separation review candidates fail to improve combat clearance
while preserving walking and jump-zone membership; none is published.
Full Leicester and Wychford construction passes, as do all fifteen control
apply/reset checks. Wychford now retains the platform and upper entrance;
its two lower entrances and jump remain omitted where the placed mill floats
above terrain. This does not create a bridge across the missing ground contact.

The compiler checks final integer jump pairs and warns about alignment,
flight-clearance and unsupported checks. Parallel authored spans share an integer
vector after placement; heights round upward like generated ledges while integer
heights and projected contacts stay unchanged. A measured engine recovery fix
allows bounded two-leg inward recovery within the initial footprint and real
floor. All 45 floor/navigation and 55 enabled compiled-navigation tests pass.
Earlier watermill candidates and their differing flight results are recorded in
the [evidence history](map-compilation-evidence.md#exported-jump-diagnostics-2026-10-08).

Receiving-material unions now batch equivalent patches. A 1,024-island component
benchmark improves from about 4 seconds to 0.1 seconds; full-map timing remains
unverified. All 166 focused compiler checks and 67 enabled native map-compilation
tests pass. The refreshed hill/river fixture has 211 receivers; 194,481 paired
native queries retain coverage and materials, with height rounding differences
up to 0.0000038146973 units. The exact-height comparison therefore does not pass;
the detailed evidence records that limitation.
Spatially disjoint receiver supports are now rejected by cached bounds before
exact intersection. The 256-floor benchmark drops from 599 to 44 ms for receiver
construction with a byte-identical descriptor; native fixtures remain unchanged.

The current ten-scene export batch (`saved-map-exports-EdJxua`) compiles and passes
native construction plus all 71 control apply/reset checks. That batch exposed a
verification-script gap: it omitted the editor's mesh-derived spline calibration.
The script now loads pinned wall meshes and runs the editor's calibration before
compilation. Fresh Wychford output (`saved-map-exports-4KKPYc`) includes its ridge
curtain without the previous missing-calibration warning and passes native
construction (153 motion areas, 15,140 sight obstacles) and all three controls.
This is descriptor-level evidence, not a complete browser bake or playthrough.
Wychford's floating church-tower approach remains disconnected, and York's
unbound appearance variants remain in their initial visual states with warnings.
All nine wall calibration/rendering and fixture-equality tests now pass. An
earlier set of three stale fixtures differed only by consolidation of fifty flat terrain
receivers into one. Audit `wall-fixture-coverage-MVpdKR` proves identical union
coverage for each height/material/receiver-property group and exact equality of
all other gameplay data before refreshing them. The reusable check is
`node refinement/check-wall-fixture-coverage.mjs`. All 67 enabled native map
compilation integration tests pass against the refreshed fixtures, including
wall routing/sight, repeated material queries and elevated lighting.
The diagnostic wall loader also matches the editor's calibration exactly for a
pinned synthetic GLB with a transformed Z-up source part, rotated section and
implicit first scene. Changed model bytes reject. Ordinary-sector route sampling
of the combined ten-map snapshots (`current-route-audit-gp9fm_nt`) now passes
1,126 directed within-sector routes. Expected connectivity comes from an
independent collision flood-fill; every returned path segment retains actor-box
clearance. The batch combines exact descriptor snapshots, using the calibrated
Wychford export, and records their hashes. It does not test inter-sector doors,
jumps, traversal callbacks or complete actor playthroughs. Counts are Wychford
84, Croisement01 16, Croisement02 6, Croisement03 24, Derby 144, Leicester 150,
Lincoln 182, Nottingham 188, Sherwood 52 and York 280. The debug audit took
531 seconds; Wychford alone spent 394 seconds in path queries. This is a
performance investigation target, not a release-game latency measurement.
Path-corridor grid queries now restrict each row to a conservative range around
the corridor sides before applying the unchanged exact cell-intersection test.
Regression tests compare the selected lines and their order with an exhaustive
rectangle scan across layers and active states, including cell-boundary cases.
This reduces candidate-cell scanning without changing geometry or actor clearance.
The same ten byte-identical descriptor snapshots pass all 1,126 routes with this
optimization (`row-filtered-route-audit-shou7alr`). The debug audit fell from
531.38 to 353.71 seconds; Wychford path queries fell from 394.18 to 262.53 seconds.
These single-run measurements show roughly one-third less audit time, not a
release-game latency guarantee. All 51 grid tests, 22 pathfinder tests and 66
enabled map-compilation integration tests pass, as does the main game build.

Rounded landing contours now repair crossed subpixel corners consistently in
the compiler and native validator while retaining their exact contours. The
southern York stair candidate passes 32 directed actor routes across eight
rotated/elevated placements on authored terrain. Giving the separate terrace and
wooden walkway their own height-aware floors restores all 130 full-York stair
routes and six controls. Partitioning holed collision before rounding now
preserves the exact stair-contact edges: all 32 routes pass with the actual
terrace moved alongside the southern stair, and 32 missing/raised terrace cases
reject. The wooden stair now has a narrower, mesh-reviewed flight and corrected
independently owned terrace/walkway contacts. Its 32 moved routes pass and 64
missing/raised neighbours reject. Both stairs and both receiving assets are
published as drafts; fresh geometry matches all sixteen tested placement files
and full York, and all ten scenes reopen. Receiving-floor mesh edge discrepancies
stay below 0.150 units, and the wooden flight below 0.087 units; rendered actor
integration remains unverified.

York's riverbank stone stair and its independently owned terrace contact are
also published. All 130 full-York stair routes and six controls pass; the stone,
wooden and southern stair copies pass 96 moved routes and reject 144 invalid
neighbour placements. Physical obstacle cleanup removes zero-width clipping
spikes before runtime conversion. Fresh published geometry matches all 24
placement exports and full York; all ten scenes reopen. The stone flight keeps
an explicit 0.577-unit sampled mesh edge-discrepancy warning.

The York market connecting stair and its independently owned raised-terrain
contact are published. All 130 full-York stair routes, six controls and 32 moved
routes pass; 48 missing/raised landing cases reject. Receiver-owned collision
rotates at its physical height while retaining a separate navigation plane.
Seam cleanup checks collision immediately beside the shared edge so an attached
wall cannot preserve a rounding strip across the entrance. Fresh published
geometry matches full York and all eight tested placement exports; all ten scenes
reopen. Full scene checks retain
288 stair routes, 84 climbing routes and 71 control apply/reset checks. Numerical
contour failures retry in local coordinates without quantization; waiting for
the clipping library's sweep limit still makes some exports slow. Rendered
integration and broader asset coverage remain open.

York's outer southeast stair and terrain-owned lower contact are also published.
A connected physical stair can now survive a pinched integer screen projection;
permanent collision that actually divides its floor still rejects. The stair
passes 32 moved actor routes and rejects 16 missing/raised authored-terrain cases;
all eight published placement exports match the tested geometry. Full York passes
130 stair routes and six control checks after correcting its separate terrain
contact. The flight has a 0.028-unit sampled edge discrepancy and up to 0.200 units
of mesh height residual. The terrain asset is a backdrop: its sampled image
coverage is not proof of a physical terrain mesh, and rendered contact remains
explicitly unverified.
All ten scenes reopen: fresh York geometry matches the tested candidate, and the
other nine maps retain their previously tested serialized geometry.

The York outer east upper stair is published and passes all 32 moved actor routes
with its separate curtain wall, and rejects 48 missing/raised wall or terrain
cases. The wall owns corrected flight clearances and landing contacts; no
scene-specific connection is added to export. Contact corners move at most
1.053 units. Sampled flight and wall-floor mesh edge gaps reach 0.327 and 0.450
units respectively. Correcting the separate terrain-owned lower contact restores
all 130 full-York routes and six control apply/reset checks. All eight fresh
published placement exports match the tested geometry, and all ten saved scenes
reopen. The ground asset provides backdrop coverage, not physical mesh evidence;
rendered contact remains unverified.

The north-garden stair's two slightly different floor planes merge into one lift.
Its initial candidate exposed a joined-flight limitation and fell back to
projected navigation. A new piecewise-floor
primitive passes bidirectional step/distance checks across rotated and elevated
joined slopes, rejects gaps and conflicting overlaps, and avoids concave-floor
shortcuts. Descriptor loading and actor movement now support these floors:
native fixtures cross both flights in both directions and retain live barrier
state. Compiler emission now preserves each flight's plane and projected slope
changes, independently clips collision, and remaps controls after cropping.
Joined and copied/rotated compiler checks pass, along with independent controls
and single-flight cropping. Native tests consume compiler-generated joined-floor
data and pass four complete routes plus four closed-barrier rejections across
apply/reset cycles. The garden stair and its terrain-owned contact are now
published as drafts. Fitting the upper flight through the shared lower edge and
upper landing midpoint changes its height by at most 0.075 units and reduces
the earlier 2.562-unit mesh overhang to 0.015 units. Mesh sampling finds support
at 1,330/1,331 points; sampled height residuals stay below 0.080 units. Synthetic
landings and authored-terrain placements pass 64 native actor routes and reject
48 missing/raised landing cases. Correcting the separate terrain contact restores
all 130 full-York stair routes and six controls. All ten scenes reopen, and eight
fresh published placement exports and full York match the tested geometry. The terrain provides
backdrop coverage; rendered contact remains unverified.

The precinct east stair and cathedral precinct's raised-terrain contact are
published as drafts. The stair's coarse walking contour is rebuilt from its own
receiving footprint; its separate neighbour owns the corrected contact edge.
Preserving precision alone did not fix the offset: the receiver corners needed
0.411- and 2.236-unit adjustments. Actual paired assets pass 32 moved actor routes
and reject 32 missing/raised receivers; synthetic landings pass another 32 routes.
Full York retains 130/130 stair routes and six control checks. Sampled mesh gaps
reach 0.274 units on the flight, 0.452 on its upper landing and 0.282 on the raised
receiver. All ten scenes reopen; eight fresh published placement exports and
full York match tested geometry. Rendered contact remains unverified.

The south-city-gate east stair is published with corrected local seams and a
placement ground height matching its lower entrance. It passes 32 copied routes
with synthetic landings and sixteen routes from new editor-terrain drops, rejecting
forty raised/missing landing cases. Full York retains all 130 stair routes and six
control checks; its lower physical receiver needs no geometry change. Flight
sampling has 731/733 mesh hits, a maximum 0.240-unit edge gap and height residual
below 0.077 units. All ten scenes reopen; eight fresh published terrain-drop
exports and full York match tested geometry. Rendered traversal remains unverified.

The west-city-wall stair and its terrain-owned lower contact are also published.
Corrected local seams pass 32 copied routes and sixteen new-drop routes, with
forty invalid landing rejections. Full York initially failed the two lower
handoffs; aligning the ground boundary in the elevated receiver's projected
coordinates restores all 130 routes and six controls. All 839 sampled flight
points hit its mesh, with height residual below 0.106 units. Sixteen fresh
published placement exports and full York match tested geometry; all ten scenes reopen.
Ground artwork coverage is verified; rendered physical contact remains open.

York's multi-entrance stairs and receiving contacts are published as drafts.
The west-lane stair retains eight entrances, and both bridge flights retain
eleven. The bridge deck owns its physical floor; the separate terraces and
terrain own their corrected contacts. The combined moved/copied assembly passes
all 1,696 complete-profile routes and rejects 64 missing/raised neighbours.
Separate bridge placements pass 800 routes; both west-lane receiving arrangements
pass 896 each. Fresh full York passes 130 stairs, 40 climbs and six controls.
Eight fresh published placement exports match the tested geometry, and all ten
scenes reopen. Draft warnings retain sampled mesh gaps up to 0.540 units and
unverified rendered integration. All 53 library stair definitions now pass the
local floor-anchor check; this does not certify every placement or visual contact.

Sherwood's central-oak stair contact correction is published as a draft. The
stair and its neighbouring ladder each pass sixteen complete-profile routes
across four rotations and two elevations. Full Sherwood passes two stairs and
ten climbs; it has no controls to test. Fresh published geometry matches all
eight tested placements and full Sherwood, and all ten saved scenes reopen.
The asset explicitly warns that its visible stair is incomplete: only 194/802
sampled points have assembled mesh within two height units, and nearest visible
height discrepancies reach 45.807 units. This is verified navigation with a
substantial unresolved visual limitation, not full asset parity.

Physical climb navigation now includes walls. Landing checks allow the outside
point and seam to occupy adjacent terrain triangles while still rejecting
missing support and hole interiors. Native wall movement retains explicit world
endpoints instead of offsetting only its screen-space approach. The Lincoln inner
gatehouse and its independently walkable plateau are now published as drafts.
Two copies pass 32 complete-profile climbing routes across four rotations and
two elevations; all 32 missing/raised plateau cases reject. Full Lincoln retains
16 stair routes, eight climbs and eleven controls. Steep wall landing checks
account for coordinate-rounding error while still rejecting actual height gaps.
The plateau retains substantial mesh warnings: sampled gaps reach 118.199 units
over its coarse footprint and 2.969 units at the edited gatehouse seam. Ordinary
and crenellated wall barriers pass traversal,
copy isolation, closure and reopening checks. These are headless movement tests,
not rendered animation certification.

The Lincoln east curtain wall's climb seams are also published as a draft.
Eight independent placements at four rotations and two elevations pass 16 stair
and 16 climb routes, and all eight raised-ground disconnections reject. Full
Lincoln still passes its 16 stair routes, eight climbs and eleven controls.
Visible wall coverage remains incomplete: only 141/542 sampled flight points
hit the mesh, with gaps up to 9.341 units. The draft warning records that limit.

Latest runtime checks retain exact landing contours through binding and foot
support instead of reducing them to single precision. The southwest turret is
now published after all sixteen rotated/elevated routes pass. Obstacle recovery
restores verified edge interiors without requiring unrelated short edges to
have a complete reconstruction; uncertain corners and true collision remain.
The latest saved-scene batch passes 288/288 stair routes, 84/84 climbing routes
and all 71 control apply/reset checks. Nottingham's road stair now has corrected
landing heights and asset-owned low-deck collision; its obsolete terrain hole
is removed. These results do not establish arbitrary-placement parity.

The west-moat tower correction is also published: sixteen rotated/elevated
routes and eight control checks pass. The compiler now retains exact contours
for obstacles formed only when separate floor pieces join. Asset-owned
clearances cover the reviewed landing extensions. Published placements and the
full Leicester export match tested candidates; all ten scenes reopen. The
tower retains an explicit warning for incomplete visible mesh coverage and
unverified rendered actor integration.

Lincoln's west slate tower now has published corrections for both stairs.
Thirty-two rotated/elevated routes pass, eight raised external approaches reject,
and full Lincoln retains sixteen passing stair routes and eleven control checks.
Published geometry matches the tested candidates and all ten scenes reopen.
Its draft records remaining mesh discrepancies; external attachments beyond
these stair contacts still need broader placement review.

The south-wall stair and its terrain/plateau-owned lower contact are also
published. Sixteen moved routes pass, eight raised approaches reject, and full
Lincoln retains sixteen passing stair routes and eleven controls. The terrain
contact has 201/205 exact mesh sample hits with four edge discrepancies below
0.373 units; the upper landing has larger discrepancies, all explicitly warned
about. Fresh published geometry matches the native-tested candidates and all
ten scenes reopen.

Lincoln's east curtain stair and its terrain/north-bailey contact are published
after sixteen moved routes, eight raised-approach rejections and full Lincoln
checks of sixteen stairs, eight climbs and eleven controls. Published geometry
matches the tested candidates and all scenes reopen. The flight has substantial
mesh gaps and the upper landing has no sampled mesh support at its authored
height; these remain explicit draft limitations, not verified visual parity.

The north-hall stair and both independently owned landing contacts are published.
Two stair copies pass 32 directed routes at four rotations and two elevations;
64 missing/raised landing cases reject. All 598 flight samples and all 205
samples at each corrected neighbour contact have mesh support. Full Lincoln
passes sixteen stairs, eight climbs and eleven controls. Published geometry
exactly matches tested candidates and all ten scenes reopen; rendered actor
integration remains unverified.

Landing binding retains a height-matched doorway when the same platform also
overlaps a different end of the flight in XY. Incompatible contacts are separated
from that landing's foot support without changing real collision or accepting
a height gap at the doorway. Receiving-floor holes and live blockers are removed
from their own landing before combining support, so an overhead platform cannot
erase a valid lower floor. A regression proves that an upper overhang alone
cannot support the lower entrance, a real ground landing restores the route,
and its live blocker still stops movement. All 23 navigation and 30 physical-lift
regressions pass, and the game binary builds.

The initial three-entrance Sherwood investigation remained unresolved; the
subsequent centered-contact correction described below now passes traversal.
Before these runtime corrections, all 48 routes failed across eight placements.
Tracing found both upper landings rejected because another shared edge was
278 units below them. The corrections restore landing support, but all six
unrotated routes still fail for both the baseline (`local-stair-placements-tDGWb5`)
and shorter-contact candidate (`local-stair-placements-Hk8MNp`). The latter now
supports the lower entrance and right upper endpoint; the left contact and
actor-sized route through the climbing surface still need correction. Near-point
diagnostic paths do not certify an actor-sized route. Complex support queries
also remain expensive: these six-route audits take roughly two minutes.
No candidate asset metadata was published from that initial investigation.

Mesh alignment audit `climb-mesh-audit-Ol6GV6` loads the hash-checked live oak model
and staged climbing surface. Its plane intersects only two of the 28 selected
rung meshes. Signed normal offsets of rung bounding-box centres range from
about +4.92 to -12.01 units across the ladder, so this is not a uniform contact
offset. The surface needs mesh-based reauthoring as well as actor-sized route
validation; relaxing endpoint checks alone would not establish usable climbing.
These measurements are alignment diagnostics, not traversal certification.
Reproduce from `level-editor` with
`node refinement/audit-climb-mesh.mjs sherwood-ladder-oak-platform gameplay-volume-101-lift rung work/needs-review/map-compile/three-door-oak-oBXE0a`.
The helper accepts other assets, lift IDs and mesh-node patterns and records
descriptor/model/staged-data hashes in its report.

The compiler and native loader now accept connected piecewise floors for ladders
and climbable walls, removing the previous single-plane restriction. Compiler
regressions cover both types at 0, 37, 90 and 180 degrees with independent copies.
A complete-sprite native fixture crosses both floor planes in both directions:
eight open routes pass and eight closed-barrier requests reject across apply/reset
cycles. Admission tests still reject mismatched seams and missing floor coverage
for stairs, ladders and walls. This enables asset-local curved climbing definitions
where each piece is a valid height surface; it does not resolve vertical surfaces,
the oak's mesh/contact reconstruction, or its actor-footprint clearance failures.
The full compiler-to-runtime placement audit (`compound-climb-placements-WPU7G9`)
passes all 72 directed actor routes through 18 exported descriptors. Of these,
64 cover connected-plane climbs across 16 descriptors: both
climb types, the four rotations, elevations 0/40, and two independent assemblies
per map. Each assembly retains both authored planes. Eight further routes cover
copied ladder/wall placements whose screen projection pinches apart while their
physical floor remains connected. The compiler now uses verified physical
connectivity for all three lift types in that case. Real permanent collision cuts
still reject traversal, retaining collision and warnings during best-effort export.
Generate these editor-only
fixtures with `node refinement/check-compound-climb-placements.mjs`, then run
`exported_climbs_support_complete_actor_routes` with `ROBIN_ASSET_MAP_DIAGNOSTICS`
pointing to the reported directory and `ROBIN_CLIMB_RHS` to a complete actor sprite.
This verifies native descriptor traversal; browser ZIP and rendered checks remain
separate gates.

Fresh published York and Sherwood exports (`saved-map-exports-vDlOcb`) pass all
132 native stair routes, 50 climb routes and six control apply/reset checks after
the runtime corrections, including the final contact-separation geometry.
Their rendered integration remains unverified.

The current library-wide audit (`gameplay-coverage-nlq6Ph`) finds 21 of 1,278
indexed assets without gameplay definitions. None is referenced by placements,
scene assets or wall-spline sources/corner assets in the ten saved scenes. The
audit now includes spline references: Wychford uses 84 distinct assets rather
than the 82 counted from placements and scene assets alone.
Seventeen missing definitions contain parts explicitly owned by initial/applied
sight states in existing assemblies. The audit now records those owner and
transition memberships per part. They cannot safely become static definitions
by copying all visible volumes: some contain both state endpoints. The remaining
four are two background-art assets, the composite Derby keep and the Nottingham
watchtower fragment; absence of a recorded sight-state dependency does not prove
their other gameplay dependencies are complete.
Croisement03 fragments 103–105 now have published independent activated-solid
drafts with asset-owned appearance bindings. Eight copied/rotated/elevated maps
pass native construction and 48 apply/reset checks with 144 thin-route probes.
All three published assets pass browser overlap export and native ZIP loading;
mesh contact and complete actor traversal remain unverified. Fragment 102 remains
separate because its assembly also owns a persistent initial blocker.
Croisement03 fragments 098/099 have independent removable-solid drafts: each starts
present, removes only its own footprint/sight/appearance when applied, and restores
on reset. Eight native constructions, 32 apply/reset checks and 96 thin-route probes
pass. Both published assets pass overlapping-copy browser exports and native ZIP
loading; these remain drafts pending full actor and mesh-contact review.
Croisement01 fragments 083 and 084 now have independent activated-solid drafts.
Each uses its own collision footprint and local control, preserving the initial
inactive state without copying the assembly's shared exclusion or remote waypoint.
Eight copied/rotated/elevated maps pass native construction and 32 apply/reset
checks, including 96 thin-route probes. Published export/reopen geometry matches
the tested stage; all ten saved scenes still reopen. Mesh contact and complete
actor routes remain unverified.
Descriptor-owned part appearance rules now reach both editor and pipeline loaders.
Insertion/save/reopen tests verify independent visibility for two copies through
all four control combinations, and conflicting model rules are rejected. These
loader tests are supplemented by a real browser check of the published fragment
bindings: independent copies (including a 37-degree rotation), save/reopen,
apply/reset, changing color/depth states and ZIP packaging pass. Both archives
load into Rust without a base datadir with two appearance controls each. This checks
resource decoding and engine construction. Both archives also pass OpenGL GPU
color and character-occlusion pixel checks across all four independent control
combinations, transition suppression and reset. Patch flags are assigned directly;
actor activation, mesh contact and complete rendered playthroughs remain open.
Vulkan verification is pending because this sandbox exposes no Vulkan adapter.
Overlapping copies also pass browser export, native ZIP loading and OpenGL color/
depth checks for both fragments: one region contains four states, with a separate
changing-pixel probe for each control. These remain rendering checks with directly
assigned patch flags, not actor-triggered gameplay tests.
The index publication path now retains these part rules in base and variant views.
The timber bridge, both ferns, fallen log, upright fences 94/95 and supplemental
wood 09/44 and Croisement01 tree 03 have published draft definitions. These counts
reflect the current asset inventory, not gameplay certification.

The asset-only physical-mesh preflight (`physical-mesh-audit-dpmilw`) checks
welded edge connectivity before collision authoring. Supplemental wood 09 has
three closed shells, wood 44 has one, the fallen log has one, and tree 03's wood
has ten. This proves edge topology only: self-intersections, nested shells,
collision decomposition and gameplay behavior remain unchecked. Wood 44 alone
has 601,032 triangles, so a direct per-face conversion would need substantial
size reduction. Both upright fences contain zero-area triangles; their geometry
needs cleanup that preserves rail gaps before solid conversion. Tree 03's crown
is open and must not silently become a solid. No new gameplay definitions were
published from this preflight. Reproduce it with
`node refinement/audit-physical-meshes.mjs <asset-id> ...` from `level-editor`.

Tree 03 now has a published physical and crown-occlusion draft.
The error-budget simplifier produces nonmanifold edges at both 0.5 and 0.1
units; the latter has no coincident float32 positions. Exact opposite-face
removal also leaves invalid edges, so those candidates remain rejected.
The alternative `--caps --decimate=0.01` audit (`physical-mesh-audit-7ARdDA`)
uses Blender collapse decimation for shells above 5,000 triangles and retains
smaller shells unchanged. The main wood shell reduces from 130,796 to 1,306
triangles; all ten shells pass closure and cap-decomposition checks, producing
3,732 capped pieces. Bidirectional vertex, edge-midpoint and face-centroid
samples measure 1.3973 units source-to-candidate and 0.8884 units
candidate-to-source (392,390 and 3,920 samples). These are sampled deviations,
not certified maximum contact errors.
The audit saves its physical input, candidate and deviation report; visual
models remain unchanged. `BLENDER_BIN` can select the offline Blender binary.

The tree definition retains 3,698 capped pieces after recording 34 numerical
slivers with total footprint area 0.00000010782893855813159 square game units.
Its open crown supplies 1,800 opaque, double-sided mask triangles independently
of physical wood; no crown solid is invented. The asset-local root receiving
probes allow up to one unit of embedding, accommodating a 0.7068-unit difference
in saved Croisement01 without a scene-specific compiler rule. Half-unit burial
binds; floating and 1.5-unit burial reject. The canopy front envelope remains an
authored approximation, and steep receiving terrain needs review.

The candidate (`wood-gameplay-wCPFLA`) passes ten rotated/elevated native
construction cases, twenty movement checks and 1,000 sight/projectile checks.
Full Croisement01 (`saved-map-exports-FeXcnT`) constructs eight areas, 3,790 sight
obstacles, sixteen doors, thirteen jump pairs, 4,180 grid blocks and 76 elevation
boundaries; all six controls apply/reset. Both masks remain present, with no
missing-definition or missing-receiver warning. One collapsed movement hole
warning remains. Publication backup: `tree-gameplay-publish-wCPFLA`. Fresh
published geometry (`saved-map-exports-R4zaCG`) exactly matches the tested
descriptor, and all ten scenes reopen. Dense collision cost and rendered
integration remain unverified; this is not full gameplay certification.

The optional `--caps` audit converts a closed shell into convex pieces with
independent planar bottom/top caps, retaining multiple solid intervals and the
air between them. It rejects crossing/coincident caps and verifies footprint
coverage and enclosed volume. Rotated/elevated sloped solids, concave corners,
stacked rails and invalid intersecting caps have focused tests. Retaining convex
cap intersections as polygons reduces the fallen log from 1,889 to 1,120 pieces
without adding triangle seams (`physical-mesh-audit-tCHJKA`). The audit limits conversion
to 5,000 triangles per shell; dense wood needs an authored physical mesh before
using this decomposition.

The fallen log now has a published physical draft with 1,103 wood volumes.
Authoring removed seventeen numerical slivers with total footprint area
0.00000025926151485335255 square game units, recorded in its review and warning.
Ten rotated/elevated exports (`log-gameplay-8tyX7n`) pass native construction,
twenty movement checks and 520 sight/projectile checks. Full Croisement03
(`saved-map-exports-T9OxL9`) constructs eleven areas, 1,736 sight obstacles,
fifteen doors, ten jump pairs and 69 elevation boundaries; all nine tested
controls apply/reset. Fresh published output (`saved-map-exports-VL3r0K`)
matches the tested descriptor except its additional draft warning. Movement
compilation still omits 39–60 collapsed grid fragments per isolated placement
and 69 in the full scene. Dense collision cost, rendered behavior and any
traversal surface/jump remain unverified. Publication backup:
`log-gameplay-publish-8tyX7n`; authoring tool:
`refinement/stage-wood-gameplay.mjs`. All ten saved scenes reopen after publication.

Fence authoring now explicitly records collinear seam faces while retaining
their edges for closed-shell validation; open surfaces and repeated vertices
still reject. The two upright fences each contain one such seam face. Their
unsimplified shells also have intersecting cap faces (`physical-mesh-audit-9e2Z1l`).
Independent shell simplification with a 0.1-unit approximate error budget
removes those intersections. This error is an appearance metric, not a certified
maximum collision displacement. Convex half-plane subtraction avoids numerical
polygon-sweep failures while retaining air gaps, footprint coverage and enclosed
volume (`physical-mesh-audit-Lok9hk`). No rail/post components are pruned or
combined into a whole-fence hull.

Upright fences 94/95 have published draft definitions with 515/1,133 wood volumes
(`wood-gameplay-DSnD9U`, `wood-gameplay-Wc4Vpa`). Across twenty rotated/elevated
placements, native construction, forty movement checks and 2,000 sight/projectile
checks pass, including 960 gap checks. Authoring records 39/67 omitted numerical
slivers with total footprint areas 0.00000018181093644842008 and
0.00000037805480106778876 square game units. Dense collision, integer-grid
fragmentation, rendered contact and traversal authoring still need review.
Publication backup: `fence-gameplay-publish-GPs4kN`.

Movement-control coverage now uses the same fixed-precision subtraction recovery
as permanent collision. This fixes a full-scene polygon-sweep failure when the
fences meet existing navigation, while retaining state bits and permanent
exclusions. Full Croisement02 (`saved-map-exports-E6S3PE`) constructs five areas,
1,802 sight obstacles, five doors, four jump pairs, 8,575 grid blocks and 57
elevation boundaries; all nine controls apply/reset. It still warns about 741
collapsed movement fragments, seven collapsed holes, seven repaired corners,
the missing wood 09/44 definitions and shrub 77's unavailable mask receiver.
These warnings and the unverified rendered behavior remain parity work.
Fresh published Croisement02 (`saved-map-exports-WtURUm`) exactly matches the
native-tested descriptor, and all ten saved scenes reopen after publication.

Supplemental wood 09/44 now have published asset-only physical drafts. Wood 09's
10,116 triangles simplify to 776 across three closed shells with a maximum
reported approximate error of 0.099032 units (`physical-mesh-audit-FyXkPe`).
Its definition retains 2,083 capped volumes after recording 29 numerical slivers
with total footprint area 0.00000024031062583832577 square game units.
Wood 44's 601,032-triangle shell produces 13,687 capped pieces at a 0.1-unit
budget; a 0.5-unit budget reduces it to 676 triangles and 1,930 capped pieces
without discarded cap fragments (`physical-mesh-audit-yIE4Rh`). Its reported
approximate error is 0.4981 units. These metrics do not certify maximum physical
displacement or rendered contact; the visual meshes remain unchanged.

Both candidates (`wood-gameplay-G6DBB5`, `wood-gameplay-ZhF0mn`) pass twenty
rotated/elevated native construction cases, forty movement checks and 2,000
sight/projectile checks. Full Croisement02 (`saved-map-exports-7GAUMW`) constructs
five areas, 5,815 sight obstacles, five doors, four jump pairs, 8,575 grid blocks
and 57 elevation boundaries; all nine controls apply/reset. Missing-definition
warnings disappear from this scene. It still reports 2,165 collapsed movement
fragments, seven collapsed holes, seven repaired corners and shrub 77's missing
mask receiver. Dense collision cost, those warnings and rendered integration
remain open.
Publication backup: `wood-gameplay-publish-1Q743w`; both published definitions
exactly match their tested candidates. Fresh published Croisement02
(`saved-map-exports-7BFBAu`) exactly matches the native-tested descriptor, and
all ten saved scenes reopen. Tree 03 has separate opaque wood and open crown
meshes; its crown must not be treated as a closed physical solid.

Croisement02 ground plants 111–123 now have published draft asset-derived mask
definitions. All 130 rotated/elevated exports construct native geometry, preserve
typed mask coverage, reject wrong-height ground and introduce no movement
collision. Their authored canopy boundary and rendered character/projectile
contact still require review. Mixed-sided shrubs and wooden objects need their
own treatment; they are not assigned empty gameplay to reduce missing counts.
Mask authoring now preserves mixed material sidedness after alpha clipping.
Shrub 55 passes ten native construction checks, but its browser review exposes
small mask/texture disagreements, including isolated discrepancies at 0° and
90°. It is published as a draft with an occlusion-review warning; rendered parity
is not established. That stage compiled Croisement02 using published
definitions with fourteen masks; native construction and nine control apply/reset
checks pass. These construction checks do not certify a rendered playthrough.

The broader shrub batch exposed a storage problem: fourteen successful draft
definitions occupy about 469 MB before publication, with one shrub alone at
127 MB. The batch was stopped and those shrubs were not published. A compact
mesh/UV/alpha representation is now implemented and staged for shrub 69: its
mask occupies 18.1 MB with 24,334 triangles, and all ten rotated/elevated exports
match the prior bitmap coverage exactly and pass native construction. This
format is published for shrub 69 as a warned draft. Full-bounds browser review
matches the compact mesh to the source model exactly, but CPU bitmap differences
remain at rotated placements (89/52/81 pixels at 90/180/270 degrees).
Migration, further size reduction and rendered verification remain before scaling
this authoring path. Footprint authoring now streams clipped triangles into a
bounded-batch hull instead of retaining the expanded mesh. Repeat/mirrored UVs
and explicit nearest base-level sampling unblock shrubs 73/74; these and shrub 68
are published as warned compact drafts after thirty placement/native checks.
Shrubs 57–60 are also published after forty placement/native checks. Identical
alpha planes shared by separate material textures now serialize only once;
this fixes a duplicated-texture size failure without changing sampling rules.
Shrub 60's origin lies in a ground gap. Published asset-local receiving points
around its footprint now bind its mask to the surrounding ground; the saved scene
exports 22 masks without that omission. Wrong-height and competing-layer contacts
still reject. Rotated CPU/GPU bitmap differences remain unresolved.
Geometry-derived
publication now supports reviewed model hashes in addition to descriptor hashes.

The earlier library-wide audit found 38 of 1,297 indexed assets without gameplay
definitions, including the composite Derby keep, several Nottingham buildings,
and York market props. At that time four were placed in the Croisement03 scene:
ferns 35/76, the stream fallen log and the timber bridge. Exporting these or
placing another missing definition in a new map omits its gameplay with a warning.
These need individual ownership/geometry review; empty scenery definitions would
be incorrect for buildings and bridges.
The reproducible `audit-gameplay-coverage.mjs` audit (`gameplay-coverage-DfT8Rc`)
finds that 32 of these assets have every part
represented in newer gameplay-bearing assets, often larger state assemblies.
Their standalone definitions still need restoring with local control dependencies
and coordinate frames; matching parts alone does not make them interchangeable.
At that audit, the four Croisement03 assets and two old ground assets had no
component-owner match. Definition counts alone do not verify masks, collision or traversal.
The two ferns now have published compact mask definitions derived from their own GLBs,
including clamped alpha sampling. Twenty rotated/elevated exports and twenty
wrong-height receiver rejections pass; native loading retains their bitmaps and
layer registration. Their closed canopy masking boundaries still need rendered
character/projectile review; publication retains explicit draft warnings.
Rendered GLB/sprite compositing exposed masks covering invisible backfaces.
Explicit one-sided mask coverage now removes those faces after placement. Both
canonical fern views have zero masked transparent pixels in the browser
diagnostic; quarter-turn views retain 6–17 mismatches. Native construction still
passes all twenty exports. Remaining rasterization and in-game review prevent
claiming rendered parity for these drafts.
Two-way coverage checks also find 5–12 rendered pixels missing from the rotated
masks. Some discrepancies are isolated beyond a one-pixel shared edge; the
browser diagnostic now reports failure instead of treating these as edge noise.
Full Croisement03 compilation with these staged definitions passes native
geometry construction and all nine state apply/reset checks. Best-effort export
now deduplicates repeated draft warnings without changing source assets or
relaxing geometry validation; strict validation still rejects repeated warnings.
The latest compact fern stage (`fern-gameplay-BL4YZw`) passes twenty placement
checks and native construction. Its full-scene export retains three masks,
including both ferns, and all nine transition checks pass. Shrubs 62–67 are also
published as compact drafts after sixty placement/native checks and exact bitmap
comparisons with their previous definitions. Croisement02 retains all six without
receiver omissions and exports 28 masks.
The next 25 foliage definitions are also published after 250 placement/native
checks. The combined Croisement02 export contains 52 masks and passes native
construction and nine transition checks. Shrub 77 is still omitted with a
receiving-surface warning in that saved scene, although its isolated terrain
placements pass. Rendered bitmap edges and approximate canopy boundaries remain
open; definition presence is not gameplay certification.
The Croisement03 timber bridge was developed from an asset-only deck candidate.
Ten rotated/elevated exports join its two end edges to matching terrain and
reject twenty one-unit height mismatches; all ten native construction checks
pass. The expanded deck-only candidate passes 40 directed actor crossings,
including 20 over a gap whose banks are disconnected without the bridge.
The solid-deck candidate now also passes all 40 actor crossings and 240
sight/projectile probes after fitting the nominal deck plane within 0.000016
height units and retaining fractional terrain fragments through compiler
clipping. The newer full-structure candidate adds mesh-derived rail/pier solids
and passes 3,120 sight/projectile checks and all 40 directed actor crossings.
Terrain retains fractional coordinates until navigation assembly, fixing the
90-degree disconnection; equivalent generated ground receivers now merge before
partitioning, fixing the 180-degree receiver seam. Full Croisement03 with the
candidate passes native construction and state apply/reset checks. Rendered
actor occlusion remains unverified. The original-scene crossing check exposed
overlapping bridge/background navigation areas. The corrected ownership
candidate removes the deck footprint from background metadata and gives the
bridge local exterior sockets. Asset ground can now opt into accepting these
sockets with `acceptsNavigationJoins`, using current placement and height.
Five interior samples have exactly one walkable owner with the bridge and none
after its removal. The saved scene passes both native actor crossing directions,
construction and all nine state controls. A moved-deck compiler regression keeps
the new floor walkable and restores the old gap. Native actor movement also
passes forty directions over twenty asset-ground exports at five rotations and
two elevations; 3,120 sight/projectile probes pass and forty wrong-height
attachments reject. The deck removes 0.000146 units of mesh-export contact drift.
Bridge and background corrections are now published, with refreshed scene pins;
both saved-scene native crossings pass using only published assets. Rendered
actor occlusion and shadows remain explicitly warned as unverified in the
bridge's draft metadata.
An unpublished mesh-derived bridge deck candidate passes ten complete native
crossings and 754 sampled receiving-seam crossings at five rotations. Adding
twelve inclined support hulls preserves those ten complete crossings, passes
twenty underpass routes and blocks sixty sampled support-foot positions. A newer
candidate adds fitted rail/brace collision and mesh-derived deck thickness and
passes those same route/foot checks plus ten landing-height rejections. Another
140 native sight/projectile checks preserve the tested wood and gaps. The latest
candidate authors 80 game-height units of upright headroom on its solid volumes.
It passes 30 native actor routes, 75 blocked support/low-clearance points and ten
landing-height rejections at five rotations. Foundation routes now detour around
low wood; deck crossings remain usable and all 140 sight/projectile checks pass.
The compiler changes pass 134 affected tests, both typechecks, focused lint and
the production build. The bridge is now published as a draft with an explicit
textured actor-compositing warning. Orthographic mesh/collision review shows
conservative timber proxies preserving the major gaps; rendered actor integration
remains unverified.

The staged composite keep preserves Derby's door permissions, room memberships
and lift endpoints; both assemblies pass 28 native stair routes and five control
apply/reset checks. New placements expose remaining rotation problems: the
initial eight-placement audit had 12 failed stair routes out of 68 tested, and
the 180-degree cases omitted one disconnected stair. Published precision settings
on the gallery/west-tower stair surfaces fix that assembly disconnection without
changing Derby's compiled geometry. The expanded candidate audit tests 80 routes:
56 pass and 24 fail, including twelve newly available routes. Failed projected
stair polygons cannot contain the test actor's 12-by-6 movement box anywhere;
full-surface clearances do not fix them. The composite remains unpublished.
An affine-footprint audit additionally finds six exact edge-on placements across
the three stairs: nonzero physical floors project to zero-area polygons. A
screen-space footprint adjustment cannot solve arbitrary rotation; traversal
needs navigation coordinates independent of rendering projection. The short
gallery stair also needs landing-overlap support, not merely a skewed footprint.
A separate landing-support route query now computes supported actor centers while
keeping the route on the stair itself; rotated seams, missing support and blocked
detours have focused coverage. Runtime loading now binds actual motion/receiver
geometry and the landing's own live collision state. Complete actor-loop entry
and exit pass at both doors of an edge-on fixture. Incompatible or unsupported
landing geometry warns without inventing walking space; broader placement and
performance coverage remains open.
Physical stair orders and normal two-door gate routes now cross an edge-on
fixture in both directions, including mid-route barrier closure/reopening.
World endpoints survive door handoffs, and collision reads the normal movement
obstacles' live state. Animation-distance stepping and position-state round trips
have native coverage even when screen positions remain stationary.

The physical region compiler combines asset-local floors and holes, solid height
slices, owner-scoped clearances and changing barriers with shared collision/state
identities. Tests cover rotated/elevated placements, headroom and fractional
barriers; the native edge-on fixture consumes emitted area/navigation data.
Export-frame clipping also operates in world space, preserving edge-on floors
and reallocating collision identities when cropping changes holes or barriers.
The main compiler shares world-space surface placement and volume-height slicing.
Its point-anchor queries now support world-space areas, preserving distinct
heights and holes at coincident screen positions. Normal exports now emit physical
navigation for compatible planar stairs, with world-space collision and shared
control identities. Receiving and material queries retain their projected
geometry; world-space anchors use the physical floor. Unsupported physical
assemblies warn and retain projected navigation. Earlier surface fitting and
region assembly still reject edge-on or disconnected cases before this emission
stage. **The refreshed keep audit still fails 24 of 80 routes.** All three keep
stairs retain projected navigation because authored door midpoints lie outside
their floor boundaries. Unsupported midpoints are 0.13–0.52 asset-local units
beyond the floor; midpoint/landing height differences reach 0.84 units. These
asset seams need correction before physical traversal can be evaluated there.
A staged seam correction now emits all three physical stairs at all eight
placements, but fails all 80 actor routes at entry: rounded landing navigation
and some receiver contours do not meet the exact door seams. This unpublished
candidate is not a replacement for the current definitions. Exact authored
landing support now survives export when its rounded contour matches one motion
region. Runtime binding accepts matching pre-grid receivers, including combined
coplanar fragments, while retaining live motion collision. Straight-edge cleanup
before rounding resolves material-partition contour mismatches; shared-edge
binding and foot support handle only bounded floating-point discrepancies.
Rotated wider-landing entry passes while real gaps and unrelated receivers reject.
The intermediate keep audit binds its landings and passes 4/80 routes. Corrected
floors also need matching asset-owned clearances: stale clearances leave thin
collision strips across the stairs. A separate unpublished clearance candidate
now passes **80/80 directed native actor routes** at 0/37/90/180 degrees and
elevations 0/40. All eight exports construct native geometry and apply/reset their
control. These are initial-state actor-loop checks, not complete-sprite or rendered
verification. The corrections now also pass 80/80 routes with independently placed
gallery and west-tower components and asset-scoped clearances. Mesh review finds
stepped treads above the smooth navigation ramps; sampled uncovered edge strips
are at most 0.24 game units wide. The component corrections are published, with
Derby/Wychford pins refreshed. All eight fresh component exports exactly match
the native-tested staged descriptors. The composite keep remains unpublished;
complete-sprite and rendered verification remain open.
All ten scenes reopen after publication, and full Derby passes all five control
apply/reset checks. All ten Derby stair definitions now pass local floor-anchor
support checks after the upper-west publication below.
A library-wide local-anchor audit finds unsupported anchors in 0 of 53 authored stair
definitions: Derby 0/10, Leicester 0/8, Lincoln 0/6, Nottingham 0/12,
Sherwood 0/1 and York 0/16. Individual audited flight floors are planar; joined
stairs may contain several planes. This checks local
definitions, not placed connectivity or actual route failures; corrections and
moved/rotated native traversal checks remain necessary across these assets.

Nottingham's southwest wall stair and independently owned wall/terrain contacts
are published. Full Nottingham passes 92 stair routes and ten control checks;
independent copies pass 192 routes at four rotations and two elevations, while
32 missing/raised wall cases reject. Fresh published geometry matches every
tested export and all ten scenes reopen. All 833 stair flight samples and 205
terrain contact samples have mesh support. The upper wall contact has only
15/205 supported samples and discrepancies up to 32.484 game units; this remains
an explicit visible-mesh limitation, not visual parity. The upper-castle assembly
also passes a stricter isolated recheck without background source terrain:
352 routes and 96 missing/raised neighbour rejections.

Nottingham's north-wall stair and upper wall contact are also published. The
candidate passes 92 full-map stair routes and ten controls. Independent copies
pass 672 routes at four rotations and two elevations; 32 missing/raised wall
cases reject. All 1,055 flight samples have mesh support; the wall contact has
184/205 hits with discrepancies below 0.109 game units. Fresh published exports
match both wall stairs' sixteen tested placement files and the combined map
candidate; all ten scenes reopen. The combined map also passes all 92 stair routes
and ten control apply/reset checks.

York's courtyard lodge stair and rear curtain-wall contact are published. Full
York passes 130 stair routes and six controls. Independent copies of the stair
and its actual lower/upper neighbours pass 32 routes at four rotations and two
elevations, with 64 missing/raised-neighbour rejections. Published geometry matches
all tested exports and all ten scenes reopen. The flight has 685/693 mesh sample
hits with gaps below 0.226 units; the wall contact has 164/205 hits with gaps below
0.580 units. These remain draft warnings requiring rendered review.

York's east riverside stair, bastion landing and terrain contact are published.
Full York retains 130 passing stair routes and six controls; two copies of the
actual wall/bastion assembly over authored raised terrain pass 32 routes at four
rotations and two elevations. All 48 missing/raised bastion or terrain cases
reject. Fresh published geometry matches all tested exports and ten scenes reopen.
The flight has 669/676 mesh sample hits with discrepancies below 0.279 units,
the bastion contact stays below 0.135 units, and all 205 terrain contact samples
are supported. The visible edge gaps remain explicit draft warnings.

Nottingham's south stair house and its terrain-owned lower contact are published.
Independent copies pass 32 routes at four rotations and two elevations, with
32 missing/raised landing rejections. New drops onto editor terrain pass another
sixteen routes and eight raised-entrance rejections. Full Nottingham passes all
92 stair routes and ten control checks. All 1,148 flight samples and 205 samples
across the corrected terrain strip have mesh support. Upper landing edges retain
an explicit warning for mesh discrepancies up to 0.310 units. Published full-map
geometry matches the native-tested candidate and all ten scenes reopen; rendered
actor integration remains unverified.

The southwest prison stair and its independently owned terrain contact are also
published. Copied assets pass 32 routes and 32 missing/raised landing rejections;
editor-terrain drops pass sixteen routes and eight raised-entrance rejections.
Full Nottingham retains all 92 stair routes and ten control checks. All 829
flight samples and 205 terrain-contact samples have mesh support. Upper landing
edges retain an explicit warning for gaps below 0.029 units. Published geometry
exactly matches the native-tested candidate, and all ten scenes reopen.

The prison-road platform is published with corrected stair seams and asset-owned
low-deck collision. Its obsolete fixed terrain exclusion is removed. Copied
placements pass 32 routes and 32 missing/raised landing rejections; terrain drops
pass sixteen routes and eight raised-entrance rejections. Eight under-deck points
remain blocked, and removing the asset restores both tested routes through its
old footprint. Flight sampling has 875/875 mesh hits and all reviewed upper landing
edges have mesh support. Full Nottingham passes 92 stair routes and ten controls;
published geometry matches the tested candidate and all ten scenes reopen.

The south curtain stair and its terrain-owned lower contact are published after
32 copied-placement routes, sixteen editor-terrain routes and forty missing or
raised landing rejections. All 1,448 flight samples, reviewed upper landing edges
and 205 terrain-contact samples have mesh support. Full Nottingham retains 92
passing stair routes and ten control checks; fresh published geometry matches
the tested descriptor and all ten scenes reopen. Rendered integration remains open.

The upper-castle stairs and their courtyard-ground, upper-wall and east-wall
contacts are now published. Real four-asset assemblies pass 352 directed routes
across independent copies, four rotations and two elevations; all 96 missing or
raised neighbour cases reject. Correcting the ground's separate physical receiver
restores all 92 full Nottingham routes and ten controls. Flight mesh discrepancies
are below 0.034 units and reviewed contact strips have gaps up to 0.692 units;
these remain explicit draft warnings. Published geometry matches the tested
candidates and all ten scenes reopen. Rendered integration remains unverified.

Nottingham's castle hall/watchtower stair is published after sixteen moved routes,
eight control checks and all 92 full-map stair routes pass. Published exports
match the tested geometry and all ten scenes reopen. Its draft explicitly records
704/789 supported flight samples, flight gaps up to 5.885 units and landing gaps
up to 6.917 units; alternate states and rendered actor integration remain unverified.
The neighbouring west stair tower and its hall-owned access contact are also
published. Two independently placed copies pass 64 actor routes and sixteen
control checks, while 32 missing/raised-neighbour cases reject. Full Nottingham
passes all 92 stair routes and ten controls. Correcting the hall's receiving edge
and its own access clearance removes the two failures exposed by the first
full-map candidate. Receiver edge shifts are at most 1.375 units, with 180/205
mesh samples supported and gaps up to 0.443 units; the flight has 618/623 hits
and gaps up to 0.307 units. These remain explicit draft limitations. Published
geometry matches tested candidates and all ten scenes reopen.
The compiler also partitions collision strips that self-intersect at runtime
precision without losing their authored footprint or changing their control
state. Nearly coincident clearance subtraction has a narrowly scoped precision
fallback that retains collision and clearance ownership.

The separate castle courtyard western stair and both receiving assets are now
published as gameplay drafts. Its independent three-asset assemblies pass all
32 routes and reject 64 missing/raised neighbours; full Nottingham passes all
92 stair routes and ten controls. Joined multi-plane motion regions now retain
precise outer boundaries, and runtime landing collision stays in double precision
through clipping and seam cleanup. The captured blocked-corner regression passes
without removing real thin collision. Published geometry matches tested candidates
and all ten scenes reopen. The stair mesh still supports only 423/929 sampled floor
points, with visible gaps up to 26.130 units; this substantial discrepancy remains
an explicit warning and an open visual-parity task. Ground and wall contacts have
smaller mesh discrepancies, up to 0.851 and 0.102 units respectively.

The Lincoln great hall and its independently owned ramp/annex contacts are now
published. The three-asset assembly passes all 96 directed routes through two
copies at four rotations and two elevations, retaining all 64 missing/raised
neighbour rejections and passing 32 control checks. Ordinary passages preserve world endpoints for their gate
approach, in-stair walks and midpoint handoff, including edge-on floors. Receiver
roundoff probes follow the actual exit direction instead of moving both axes
equally. The two passage midpoints are corrected to the stair boundary, with
6.840/11.933-unit shifts; no door is removed. The existing stair-flight review
has 566/566 mesh sample hits. Full Lincoln passes sixteen stair routes, eight
climbs and eleven controls. Fresh published geometry equals tested candidates,
and all ten scenes reopen. The ramp's corrected contact edge lies up to 0.299
units beyond its mesh; the annex contact has discrepancies up to 0.473 units.
These remain explicit draft limitations, with rendered actor integration still
unverified. Fresh exports of
the ten saved scenes retain all 288 stair routes and 84 climbing routes with
the runtime fixes.
The Nottingham road stair is also published after sixteen moved/elevated routes,
eight raised-ground rejections and all 92 full-map stair routes pass. Its low-deck
collision now follows the asset, with 80 units of authored upright headroom.
Eight placed under-deck points block correctly, and actors cross the former
platform footprint after removal. Foundation clearance moves by at most 0.625
units; preserved terrain now retains matching fractional obstacle contours.
Mesh review finds 753/829 floor samples supported, with discrepancies up to
2.504 units. The draft retains an explicit rendered-integration warning.
Published exports match the tested candidates and all ten scenes reopen.
The northwest tower stair and its keep-owned receiving edge are now published.
Sixteen moved stair routes and eight control checks pass; raising the tower away
from its ground rejects all eight connections. New drops use the lower entrance
as their ground reference. Full Leicester passes sixteen stairs, 22 climbs and
twelve controls. The platform edge moves at most 0.879 units; 188/205 mesh samples
hit its visible surface, with remaining margins below 0.195 units. The stair's
visible mesh remains incomplete and is explicitly warned about. Published
descriptors match the tested candidates and all ten scenes reopen.
Near-coincident receiving-footprint comparisons now use fixed-point clipping,
fixing an export crash exposed by this full-scene check.
The great keep stair is also published after sixteen moved routes and sixteen
control checks. All eight raised-ground cases reject; new drops use the lower
entrance height. Full Leicester retains sixteen stair routes, 22 climbs and
twelve passing control checks. The floor has 676/677 sampled mesh hits, with
the remaining sample 0.032 units from the mesh. Published exports match the
tested candidates and all ten scenes reopen. Fractional receiving contours now
use a physical-area threshold instead of the integer-grid minimum, fixing a
separate southwest turret export error. That turret is now published after
sixteen moved routes, eight raised-ground rejections and a full Leicester check
of sixteen stairs, 22 climbs and twelve controls. Its floor has 694/695 mesh
sample hits, with the remaining point 0.033 units from the edge. Published
exports match the tested geometry, all ten scenes reopen, and the draft retains
an explicit rendered-integration warning.
The first church-side tower candidate emits both physical flights at eight
placements. It initially passed only 24/32 routes because rounded landing-hole
boundaries blocked the upper flight at 0/90 degrees. The compiler now retains
matching precise hole contours; native loading validates their grid footprint and
physical landing collision preserves their obstacle identities and live states.
All 32/32 routes and eight control apply/reset checks now pass, and eight raised
external approaches still reject. The landing review still finds incomplete mesh
coverage; 38 missing-floor foot samples are visible near the lower entrance in
the default view. The church and adjacent terrace corrections are now published
as drafts with an explicit visual warning. The terrace owns its walkable top and
clearance; its reviewed seam extends at most 0.233 units beyond the visible mesh.
Together, the independently placed assets pass 32/32 directed routes at four
rotations and two elevations, with all sixteen missing/raised-terrace cases
rejecting. Full Leicester retains 16/16 routes and twelve control apply/reset
checks. Runtime landing clipping removes bounded floating-point strips on exact
shared stair edges while preserving real thin obstacles. The earlier diagnostic
that removed the blocker is not a publishable floor definition.
Landing mesh checks now include other components in the same asset, since the
visible floor and receiving surface can belong to different parts.
The east-wall turret stair and its terrace contact are also published. Two
independent copies with authored external landings pass 32/32 routes and reject
32 missing/raised landings. The turret with the actual terrace passes another
16/16 rotated/elevated routes and sixteen disconnected cases. Full Leicester
retains 16/16 routes and twelve control checks. The upper landing correction is
limited to the stair width; its seam has complete sampled mesh support, while
short connecting edges extend at most 0.156 units beyond mesh. The terrace edge
has a maximum 0.268-unit discrepancy. Both drafts retain rendered-integration
warnings. Published descriptors equal the tested candidates and all ten scenes
reopen; the isolated turret/terrace pair contains no switches.
The east-moat tower stair is now published after 16/16 moved routes and another
complete Leicester check: 16/16 stair routes, 22/22 ladder/wall routes and twelve
control checks pass. Its upper landing required a reviewed 2.056-unit edge
adjustment; the authoring tool now rejects nearby edges that miss the doorway
and adjusts only edges actually selected. Exact obstacle recovery also preserves
the lower landing's rounded notch. Fresh exports match the tested geometry and
all ten scenes reopen. The stair mesh remains incomplete, with missing samples
occluded at all four reviewed rotations; landing discrepancies reach 0.387 units.
The east-moat ladder now also has published endpoint and landing corrections.
Physical ladder navigation fixes its narrow projected corridor: all 16/16
complete-animation routes pass at four rotations and two elevations. Landing
surfaces retain their precision until final grid rounding; ladder movement
orders use exact world endpoints while transition animations keep their own
posture and membership effects. Compiler checks require a landing to support
both the outside point and seam before enabling physical traversal. Unsupported
definitions retain projected navigation with a warning when their landing
heights or coverage still disagree.
The expanded local-anchor audit finds 1/8 ladder definitions and 5/9 wall
definitions with unsupported floor anchors. Those counts do not check landing
height compatibility or certify placed routes. Physical wall compilation and
runtime traversal are supported, but these wall asset contacts remain unfinished.
The keep-annex candidate remains unpublished. Surface-based clearance fixes all
eight previously failed 90-degree routes, so the original 32-route placement
audit now passes. An expanded 25-angle sweep exposed a separate 150-degree
entrance-rounding error: a supported exact point was discarded after rounding
outside its roof. Physical lift endpoints now retain their world coordinates
through compilation and receiving-floor binding. All 50 expanded placements
compile, with 200 missing/raised-neighbour rejections. Native traversal passes
200/200 routes after ordinary physical-floor movement integration. The previous
196/200 result exposed four approach receiver mismatches at 150 degrees.
The native trace located these before entry: projected move-box recovery moved
the actor off its supporting roof. The exact outside point supports the normal
inset world-XY movement footprint, but not its projected rectangle; the full
world-XY source box still has a small uncovered corner. Walking-source recovery and
approach clearance now operate in physical space for point moves and gate
approaches on one bound floor.
Physical walking floor queries now provide precise routing and bounded source
recovery with holes and current obstacles. They distinguish the inset walking
footprint from the full standing box and expanded recovery box. Actor dispatch,
stepping, live barriers and compatible-height neighbour collision now use them.
The entire problematic projected landing has
no valid center for any of those three footprints, so relocating its outside
anchor cannot solve the projected clearance problem.
The loader now binds complete ordinary receiving floors separately from trimmed
stair landing contacts, using only exported motion and receiver geometry. Both
copied 150-degree roof approaches have supported recovery positions about 1.15
world units away on these full floors. Both elevations now pass all eight focused
routes, and the expanded sweep passes all 200. Same-floor physical dispatch also
checks an explicit destination's layer and sector, preventing a goal on an
overlapping upper floor from being accepted as a move on the lower floor.
Unspecified local destinations and the existing gate handoff remain supported.
Seek/line movement, cross-floor
routing and ordinary soft repulsion still need integration. Projected
collision cannot reconstruct an exactly edge-on ordinary receiver; that case
still needs explicit world collision data.
Reliable new-map placement, rather than matching an existing layout, is the
acceptance criterion.
York's central-lane gable-house climb and separate lean-to roof remain unpublished.
The two-unit asset-local entrance shift and precise physical receiver-plane fit
now pass **200/200 complete-character routes** across 25 angles, two elevations
and two copies. Compilation rejects all 300 disconnected-neighbour cases.
The seam shift avoids a real roof-corner shortfall; fitting the receiver anchors
in double precision avoids a separate translation-sensitive landing gap. No
floor padding or wider acceptance tolerance is used. A captured binding test
reproduces the old failure and continues to reject a real 0.02-unit height gap.
The fresh full York candidate still passes only **38/40 climbs**: the lower seam
lies inside shared exclusion-4 blockers owned by the rear gable house and the
raised terrain. Their collision ownership must be reviewed so terrain exclusions
do not depend on the old building layout. A separate compiler fix now admits
precise hole-boundary seams on raised receivers, while rejecting hole interiors;
the full York descriptor is unchanged by that fix. An unpublished physical-deck
experiment now passes all **40/40 native climbs** after fixing recovery of
fractional edges when rounding joins separate solids. The previous 38/40 result
came from a rounded collision wedge at the lower handoff, not insufficient mesh
width. The regression preserves unrelated collision through four turned and
translated placements, without changing the integer navigation contour or actor
tolerances. The experiment still raises omission warnings from 8 to 45 and is
not ready for publication. Correcting the east-riverside curtain-wall and market
southwest connecting stair movement volumes to match their authored flights now
raises the experiment's stair audit from **126/130 to 130/130 routes**, with
**40/40 climbs** and all six geometry controls passing. The two flight-volume
corrections are published independently, preserving the other library collision
choices: the published candidate retains eight omissions, **130/130 stairs**,
**40/40 climbs** and all six controls. Each asset also passes **32/32 native
routes** across copied, rotated and elevated placements, with 48 rejected
disconnected-neighbour cases. All ten saved scenes reopen after publication;
fresh published York geometry matches the native-verified candidate. These
checks do not establish rendered parity. The experiment also exposed
and fixed a collinear obstacle-triangulation crash, with coverage preserved by a
captured regression. The mesh review has 596/602 sampled
flight hits, maximum nearest-mesh height residual 0.872101 and maximum uncovered
edge distance 0.039980; rendered traversal remains unverified. Fresh published-scene
regressions retain all 84 climbs and 192 stairs across five saved maps. Failed
native audits retain the actual movement-order route, bound landing geometry and
initial collision states; `audit-lift-support.py` measures contact support.
Passing the placement sweep does not certify the remaining full-scene or visual
behavior; the gable, lean-to and broader physical-terrain candidate remain unpublished.
An additional unpublished doorstep experiment makes the stone tower and narrow
timber house in York's west-town lane own the physical tops of their existing
entrance volumes. It restores two building entrances, two receiving bindings and
one light binding in the physical-terrain candidate, reducing omission warnings
from 45 to 40 without new feature omissions. The floors expose matching exterior
sockets to independently authored terrain: 32 copied/rotated/elevated placements
compile, and 64 missing/raised-terrain cases reject the connection. The initial
native receiver sampler covered only 0 and 37 degrees. A subsequent normal
movement-dispatch audit exposed incorrect floor selection on every incoming
approach: a projected goal over the doorstep was unprojected onto the ground
below. Dispatch now requires the destination receiver to belong to the selected
physical floor. All **64/64 directed approaches** pass across all four tested
angles, two elevations and copied placements, using a 6-by-3 half-diagonal.
A subsequent complete-character-sprite audit also passes **64/64 approaches**;
it accounts for the zero-distance stop animation's 0.01-unit reserved endpoint
only after that animation's order chain finishes. Building entry/exit coverage
initially passed only 4/32: 22 exits selected the destination floor prematurely,
then an elevation crossing switched them back to the ground underneath.
Compiled physical-floor exits now bind the receiver at their actual midpoint.
Generated receivers also carry independent, well-spaced plane anchors: clipping
can leave nearly collinear leading vertices, which previously changed the
loaded slope after translating a copy. That correction improved round trips to
28/32. The remaining four source-authorization failures are now resolved by
preserving ordinary receiving contours before integer rounding and supporting
feet across actual height-matched neighbouring floor edges. All **32/32 complete
building round trips** now pass, alongside both sets of **64/64 approaches** and
64 rejected missing/raised-terrain connections. Neighbour support retains holes,
live blockers and floor ownership, and rejects gaps, corner-only contact and
height/layer/sector mismatches. Full world-space cross-floor routing still needs
integration. All six full-map geometry controls apply and reset, and fresh
traversal checks pass all **130/130 stair** and **40/40 climb** routes with
complete sprites and the contour/support revision.
Rendered contacts, wider terrain integration and 15 feature omissions still
need review before publication.
The receiving-plane loader now propagates f32 rounding through both the plane
anchors and each receiver vertex. Steep valid planes no longer fail loading
merely because assets are translated; the regression still rejects explicit
height mismatches. York's southwest precinct climb and receiving platform are
now published as drafts: 32 copied/rotated actor routes and 48 disconnected
receiver checks pass. The climb owns the clearance through its separate crest;
it does not depend on surrounding buildings to clear its upper exit. Full York
passes 130 stair routes, 40 climbs and six control checks. Receiving-floor mesh
gaps remain below 0.282 units in this review; rendered integration is unverified.
York's northern climb, receiving curtain wall and adjacent boundary wall are
also published as drafts. Their asset-local openings now follow the corrected
flight and upper contact, including the climb's own crest. All 64 actor routes
pass across copied, rotated and elevated placements with integer and fractional
origins; 64 missing/raised receiver checks pass. Full York retains all 130 stair
routes, 40 climbs and six control checks. Mesh gaps remain up to 0.413 units,
and rendered integration remains unverified.
The York scaffold ladder correction is published. Its upper platform notch now
meets the physical seam, and both approaches follow the flight centerline while
retaining their outside anchors and inside heights. All sixteen rotated/elevated
climbs pass. Full York passes forty climbs, 130 stair routes and six control
apply/reset checks. Published exports exactly match the reviewed candidates,
and all ten scenes reopen. The flight and upper landing retain mesh discrepancies
up to 0.711 and 0.537 game units respectively, with an explicit rendered-review
warning. Sherwood's three-entrance ladder is the remaining unsupported ladder.
The Sherwood central oak ladder correction is now published. Ordinary physical
landings retain matching pre-grid boundaries too, fixing lost seam support.
The asset clears only the corrected platform extension and owns a matching
non-solid receiver, retaining its original collision and inside waypoints.
Fresh published placements pass sixteen climbs; full Sherwood passes ten climbs
and two stair routes. The lower platform seam extends up to 1.643 units beyond
its mesh and the ladder has incomplete visible coverage; both remain explicit
draft limitations requiring rendered review.
The west-treehouse ladder correction is also published after sixteen moved
climbs and full Sherwood's ten climb/two stair routes pass. Its upper receiver
matches the corrected platform, the clearance is limited to the added strip,
and the platform hole remains intact. The upper seam's 3.381-unit mesh discrepancy
is an explicit draft warning. Published exports match the tested candidates and
all ten scenes reopen; rendered integration remains unverified.
The three-entrance Sherwood ladder oak still needs rendered platform
and climbing-surface review: its authored platform hole excludes mostly
mesh-supported upper approaches, while its rung centers diverge from the current
climbing plane. The reconstructed plank footprint and centered left contact
are now published as a draft, retaining all three connections.
The first three-entrance candidate retains all doors and passes 36/48 moved
routes, with twelve 90-degree failures. A separate compiler correction accepts
ladder seams on platform-hole edges while still rejecting hole interiors.
The focused native fixture passes both directions; a broader rotation check
exposed a receiver mismatch also present without a hole. That mismatch is now
fixed: ladder exit animations retain the physical seam instead of snapping back
to its integer waypoint, and all eight rotated fixture routes pass. These
earlier complex candidates did not establish complete traversal.
The 2026-10-08 recheck (`local-stair-placements-Vxn5ch`) still passes only 36/48
complete-sprite routes; all twelve 90-degree routes fail. All eight placements
retain three doors but lack physical navigation at the left upper landing, and
eight raised-ground cases reject. The pinned-model audit finds only 2/28 rung
parts crossing the current climbing plane, with a maximum center-plane distance
of 12.012175 units. This remains an asset geometry/contact review, not evidence
that an engine extension is needed.
The centered-contact correction (`three-door-centered-contact-pu1uxP`) now passes
all 48 complete-sprite routes at four rotations and two elevations
(`local-stair-placements-OTv6aK`), with physical navigation at every placement.
All eight raised-ground cases still reject. Full Sherwood
(`saved-map-exports-yQPQJi`) passes native construction, ten climbs and two stairs.
Its left landing allowance extends up to 2.366 game units beyond sampled mesh;
rung alignment remains incomplete. These limitations are explicit asset draft
warnings, not a rendered-contact parity claim. The correction changes asset-local
surfaces, clearances, a receiving volume and the left upper approach; it requires
no engine change. The hash-bound replacement is retained in
`refinement/catalogs/sherwood-ladder-oak-centered-gameplay.json`.
All 84 climb routes in the ten saved-scene exports now pass, as do all 71 control
apply/reset checks and Derby's 28/28 stair routes. The published tower matches
its native-tested candidate; Leicester retains 22/22 climb and 16/16 stair
routes. The ladder mesh remains incomplete: three sampled feet are exposed at
one reviewed rotation, and landing discrepancies reach 0.141 units. Its draft
retains an explicit rendered-integration warning.
The tower also retains all 16/16 rotated stair routes. Clipped landing collision
keeps its computed precision through route and footprint queries; rounding those
small intersections back to runtime floats could otherwise create invalid polygons.
Both Derby southwest-postern ladders now have published floor and landing seam
corrections. All 32/32 complete-animation routes pass at four rotations and two
elevations; eight raised-ground approaches correctly omit the disconnected lower
ladder while retaining the upper one. Full Derby retains 4/4 climbing routes,
28/28 stair routes and all five control apply/reset checks. Published placement
descriptors match the tested candidates, and all ten scenes reopen. Both ladders
retain explicit mesh/rendering warnings: uncovered sampled feet and heads remain
visible at reviewed rotations. These checks do not certify rendered integration.
The Lincoln courtyard shed now publishes a sloped ladder/roof intersection and
an approach point farther onto the existing roof. Its new-drop ground height is
the lower entrance, rather than the buried foundation. All sixteen moved/elevated
complete-animation routes pass, and eight raised-ground entrances reject.
Full Lincoln retains all eight climb routes and eleven control checks.
The roof approach differs from the mesh by up to 0.196 units, and the ladder mesh
remains incomplete; the draft explicitly retains these visual gaps.
Physical ladder eligibility now also recognizes raised receiving volumes with
unambiguous point-anchor bindings. A native fixture passes both climb directions;
an underlying navigation hole still rejects support. Segment-bound receivers
remain outside this additional eligibility path. Lincoln's terrain and neighbouring
plateau collision now publish matching ground-contact corrections, with all 205
samples covered by the mesh. A three-unit sideways lower approach avoids the
annex collision without removing it. The published shed retains all sixteen
moved/elevated routes; full Lincoln now uses physical ladder navigation and
passes eight climbs, sixteen stairs and eleven control checks.
Precise walking-area boundaries survive export for raised physical landings;
native validation requires the same integer-grid footprint. Binding clips that
boundary to the actual receiver and retains live obstacles. This fixes rounding
gaps without inventing missing floor. Rendered climbing remains unverified.
The east hall's upper stair now has published floor/landing seam corrections and
passes 48 directed routes across four rotations and two elevations. Full Derby
passes 28/28 stair routes and all five control checks with this correction.
The lower-stair correction and mesh-supported Derby terrain contact are now also
published. Both stairs pass 64/64 routes on authored terrain, and eight raised,
disconnected ground entrances reject. Preserved terrain boundaries now retain
their exact receiving contours, restoring full Derby's 28/28 stair routes and
five control checks. Landing-support routing considers only geometry within a
footprint of the stair bounds; the final Derby route audit takes about eight
seconds. These are initial-state actor checks, not complete rendered parity.
The lower-east curtain stair is also published after 16/16 placement routes,
eight disconnected-ground rejections, complete mesh sample coverage and another
28/28 full Derby route check. The east-bailey stair and its mesh-supported terrain
contact are now published too: all 16 rotated/elevated routes pass, eight raised
entrances reject, and full Derby passes 28/28 routes plus all five controls.
All 205 terrain-contact samples have mesh support; the largest terrain edge shift
is 1.165 units. Fresh published exports match the tested candidates and all ten
scenes reopen. Stair handoffs query the actual seam, with a bounded floating-point
boundary probe, instead of the outside waypoint. The upper-west stair now has a
published mesh-derived floor, local platform seam, material plane and clearance.
This replaces a floor up to 14.84 units above exposed tread centers. The corrected
flight emits physical navigation at all eight placements and passes 16/16 routes.
Entry callbacks retain world position and physical floor ownership before the
next walking order is installed. A mesh-supported terrain contact restores full
Derby's 28/28 routes and all five control checks. The ground strip has 205/205
mesh hits; the revised platform edge has 41/41. Flight samples hit 751/755 points,
with uncovered edge strips at most 0.364 units wide. Published exports match the
native-tested candidates and all ten scenes reopen. Complete rendered actor
verification and broader placement/state coverage remain open.
The upper gatehouse stair is now published after a mesh-reviewed 2.567-unit
corner correction. Its eight placements pass 16/16 actor routes and eight
control apply/reset checks; full Derby passes 28/28 routes and all five controls.
Runtime landing binding now preserves an exact receiver whose rounded footprint
fits inside a joined movement region, even when another receiver occupies the
rest at a different height. Overhanging or unsupported receivers still reject.
Fresh published exports exactly match the tested candidates and all ten scenes
reopen. These remain initial-state traversal checks, not full rendered parity.
The lower-west access stair now has a published mesh-reviewed seam correction.
Two independent copies connected to synthetic landing assets pass 32/32 routes
when landing boundaries are preserved; 64 missing/raised-landing cases reject.
Ordinary collision-split landings now also pass 32/32 routes: the compiler traces
their emitted edges back to unambiguous source edges, clips the recovered contour
to the original coverage and verifies identical grid rounding. All 64 negative
cases still reject. Ambiguous contour recovery retains the existing fallback;
these tests do not certify every split topology. Both external Derby contacts
are now corrected in their owning assets. All 205 terrain-strip samples have mesh
support. The wall strip has 167/205 exact mesh hits; the remaining points extend
at most 0.229 units beyond its mesh. An explicit 0.25-unit authoring review bound
accepts this discrepancy without changing compiler/runtime connection tolerances.
Full Derby passes 28/28 routes, native construction and all five controls;
fresh published geometry matches the tested candidate and all ten scenes reopen.
Physical transitions
now wait at a reached world target until animation completion, or preserve
unfinished distance in the next movement order. An edge-on actor-loop test
covers both cases. Local point Move requests now resolve an invertible stair
projection to world destinations, validate current goal support, and annotate
generated movement orders with the physical floor. Ambiguous edge-on clicks,
off-floor goals and goals inside active barriers reject. Seek and line requests,
edge-on mouse destination selection, physical-distance transition placement,
broader transition/seek choreography and multi-door coverage remain unfinished.
Movement-source authorization now checks physical floor height and live footprint
support before projected extraction can relocate the actor. Unsupported physical
sources warn and reject; recovery to a nearby supported world position remains open.

Physical stair movers now include hard collision from actors on explicitly bound
landings. Checks require matching sector, layer, receiver footprint and height;
overlapping radii count even when the neighbour's center lies outside the stair.
Native coverage verifies waiting and resuming at a blocked endpoint and rejecting
an actor at a different height. Closing barriers now check full actor footprints
across bound stair/landing sector boundaries in both directions; native tests
verify crushing, clear-footprint rejection and preserved unaffected orders.
Reciprocal neighbour avoidance for ordinary landing movers and soft repulsion
remain unfinished, alongside broader placement/state coverage.
Water-particle emission now shares ordinary movement's animation-distance
threshold and cadence, with an edge-on actor-loop check of particle world
positions and layers. Complete rendered movement-effect coverage remains open.
Best-effort export now warns instead of aborting on collapsed mask boundaries or
collision-split stair regions, retaining independent usable features. These
fallbacks do not repair the missing traversal.

The watermill now publishes asset-owned body collision and a platform clearance
projected after placement onto its foundation navigation plane. Twenty-four new
placements at eight rotations and three elevations retain all three entrances,
the jump connection and both masks; native checks pass 48 actor crossings and
24 blocked body points. Leicester's complete compiled geometry remains unchanged.
Wychford's existing mill still sits above its terrain approaches; no connection is
invented across that gap. New drops use the correct foundation height. Native
mouse queries also now reject loaded movement-obstacle interiors. These focused
checks do not establish every platform route or complete asset coverage.

Latest editor validation (2026-10-06): 861 tests passed, two skipped. Fresh
descriptors for all ten saved scenes pass native loading and all 71 control
apply/reset checks; Derby retains 28/28 stair routes. The game
build and native stair/control checks passed. Wychford loads with its spline wall
and passes control apply/reset. The terrain-junction correction passes the
synthetic four-triangle fan, all 48 crossings in the reduced Wychford case, and
182 movement tests (five skipped). The updated game build passes. The full
Wychford receiving-seam audit with the published stair precision settings passes
17,482 directed actor crossings over 8,741 eligible pairs. This samples
initial-state routes; it does not verify every
possible route, control state or feature category.

A fresh all-map batch exposed stale Lincoln spire appearance bindings, now
repaired and published. Native baseline construction and all eleven control
apply/reset checks pass. Moving the hall/spire leaves unresolved neighboring
receivers. The refreshed ten-map descriptor batch passes native construction
and apply/reset for all 71 compiled controls, but retains omissions. The five
Lincoln static props now have reviewed scenery-only definitions: their nearby
collision belongs to other assets. Lincoln's geometry is unchanged, all eleven
controls pass apply/reset, and twenty rotated/copied prop assemblies retain the
underlying terrain without adding collision or floors. All ten scenes in that
batch reopened with gameplay definitions for their placed assets. An intermediate
audit subsequently identified four uncovered placements in Croisement03; those
have since received definitions, as reflected by the current library audit above.
Neither check certifies the completeness of existing definitions. Wychford retains an unused great-keep
library reference without gameplay. The separate calibrated Wychford export also
passes native construction and all three control apply/reset checks, retaining
the church-traversal height mismatch and, before the woodland-bank fix below,
five physical-receiver binding omissions.
The calibrated descriptor has 146 movement areas, 21,323 sight obstacles and
17 masks. An earlier calibrated Wychford browser ZIP passes native mod discovery, image decoding and construction
without a base datadir, including its editable scene and appearance resources.
This remains short of full feature coverage or rendered actor/state parity.

The west-tower reveal now has a published terrain receiver probe. Derby's
compiled geometry is unchanged; Wychford gains its third control and two masks,
with native construction and apply/reset verified. Its elevated entrance and
the church traversal remain unresolved. The latest baked Wychford ZIP predates
this attachment change.

The woodland bank now publishes a bounded terrain attachment. Leicester's compiled
geometry is unchanged; both Wychford copies bind, with 74 sampled actor crossings
across 37 affected receiver pairs passing. Twelve new placements bind and four
out-of-reach cases reject. Following the footbridge and edge-bank publications
below, the watermill was the only remaining omitted physical receiver in the
uncalibrated Wychford descriptor. The subsequent watermill draft supplies that
floor directly; its lower ground connections still lack terrain contact.
The latest baked ZIP predates these changes.

The edge bank now publishes bounded receiver and mask attachments based on its
own height range. Leicester compiles unchanged; Wychford gains one receiver and
two masks. Thirty sampled native actor crossings pass, as do native construction
and all fifteen control apply/reset checks across both maps. Twelve new placements
bind and four beyond the finite reach reject. All ten saved scenes reopen with
updated pins; one bank mask outside the export frame remains correctly omitted.

The published east-village footbridge now owns its sloping walkable deck and both
end sockets. It
passes 30 directed native actor routes across new terrain landings at five
rotations, including complete bridge crossings. It rejects raised, mismatched
landings. The published Leicester ownership migration preserves doors, controls and
sight obstacles exactly, passes its lower actor crossing and removes the old
walking footprint when the bridge moves. Its upper neighbour remains connected
through controlled drawbridge passages. Scene pins are refreshed and all ten maps
reopen. Fresh exports from the published definitions pass native construction and
all fifteen control apply/reset checks across Leicester and Wychford. Wychford's
current bridge ends sit above their terrain receivers and still require appropriate
landings or placement; the new deck does not silently bridge those height gaps.

Sloped asset sockets now connect to authored terrain using the shared edge's
heights, avoiding false rejection from an offset height probe. Matching and
mismatching slopes are checked at five rotations; ten directed native actor
routes across the matching deck/terrain connections pass. Rotated coplanar
terrain clipping also rejects floating-point dust before it can abort export.

Assets can author a placement ground height for new drops. The church-side
tower now uses its lower stair approach, verified at three elevations and four
rotations. Existing placements and Leicester's compiled geometry are unchanged;
this does not repair Wychford's already-elevated approach automatically.

| Map information | How the editor constructs it | Current evidence / gap |
|---|---|---|
| Background and minimap | Render placed models and textures, then downsample. | Browser bake and native ZIP decoding tested. |
| Character occlusion | Bake 16-bit depth from geometry, including paired appearance states. | Static/changing GPU fixtures pass; complete scene compositing remains open. |
| View/projectile masks and masking boundaries | Transform and rasterize local coverage; rebuild receiver, obstacle and state links. | Compiler/native state fixtures pass. Library coverage and complete visual integration remain incomplete. Depth alone does not replace these masks. |
| Walkable regions and layers | Transform surfaces and heights; join matching boundaries, coplanar surfaces and authored multi-plane regions. | Synthetic joins and sampled map routes pass; complete connectivity/traversal remains open. |
| Movement collision and openings | Intersect placed solids/contours with receiving planes; apply asset-owned clearances and optional per-volume upright headroom. | Compiler/runtime fixtures pass, including slopes, raised solids and spline headroom. Recovered geometry still needs review. |
| Spline walls | Measure pinned source meshes and deform local surfaces, collision, material contours, lighting, mask coverage and spatial sound emitters with source rotation, trimming, straightening and path placement. | Geometry comparisons pass 24 combinations; Wychford native loading/state checks pass. Material ownership passes moved/repeated, turned, sloped, corner and fractional-origin compiler checks; native ground/obstacle queries pass for three repeated sections. Explicit point/segment probes pass repeated, rising and curved compiler checks and native-fixture equality; native ambience queries pass. Disconnected lighting probes survive clipping independently and match the native ambience fixture. Automatic curved/rising lighting binds from source surfaces and passes native elevated-layer shadow queries. Spatial sound placement and acoustic settings pass native construction. Broader receiver-material, playback and traversal checks remain open. Repeated masks pass native bitmap, character/projectile boundary, altitude and obstacle-isolation checks; curved/cropped coverage passes editor tests. Longitudinal mask probes preserve bends and reject missing or competing receiving layers. A surviving explicit probe also preserves masks whose point anchor was trimmed. Disconnected receiving probes survive trimming without artificial connections and match the native mask fixture. Disconnected open character/projectile application boundaries now export as separate masks, retaining their independent ranges and sharing coverage; view and obstacle rules are emitted once per repeat. Editor/native fixture equality and native application-range queries pass, including moved curved/rising compiler checks. Closed application contours now split into independent islands, with moved/bent/rising compiler comparisons, native character/projectile queries and editor export/reopen equality passing. Disconnected sound crops retain one emitter per repetition using exported fragment indices; native distance and noise queries exclude the clipped gap. Cropped point anchors retain masks when a unique asset-owned floor survives; external or ambiguous receivers still require explicit probes. Stateful sources and global sounds remain unsupported and emit warnings. |
| Navigation graph and fast-find grid | Compile native nodes, clearance configurations, distances and state-constrained links at export; bind grid/spatial structures at mission start. | Native detours and state switches pass; Derby previously passed 144 sampled ordinary routes. Uses the stock 6×3 half-diagonal; large graphs use a versioned stream with 32-bit link indices. Wychford passes 98 sampled routes and a narrow-contour actor round trip after the shortcut-clearance correction. Full actor/stair and large-scene coverage remains open. |
| Sight/physical obstacles | Transform local shapes, heights and physical/opaque flags. | Native initialization, apply and reset tested. Eight editable ZIP round trips pass 320 live guard visibility assertions for initial/alternate barriers across independent copies, rotations and elevations. Asset ownership coverage and complete encounters remain incomplete. |
| Projection/elevation receivers | Derive height planes and crossing boundaries from placed receiving surfaces. | Fractional seams, slopes, copies and sampled actor crossings tested. Generated contours are checked at native float precision; pinched contours are triangulated and zero-area pieces warned/omitted. The refreshed editor terrain export passes native polygon, hill/ford/crop and route-sampling checks; complete placement coverage remains open. |
| Doors, gates and locks | Transform endpoints; resolve current neighbours and local initial/alternate permissions. | Compiler/runtime fixtures pass; incomplete assets still warn. |
| Building interiors | Connect entrances in each asset-local room automatically; use editor links or passage sockets between assets. | Separate, joined, moved, rotated and copied assemblies and editable ZIP round trips tested. |
| Stairs, ladders and climbable walls | Assemble local traversal surfaces/sockets and endpoints; prepare actor-sized native approaches and passage-state conditions at export. | Native stair rotation/copy checks and complete-character-animation tests cover changing climb barriers, independent copied controls, closure during climbing and 72 interior reopening cases. Twenty-four fresh ladder/wall entrance fixtures reject an already-active barrier at either end and traverse both ways before applying and after resetting it. Runtime checks a cached availability flag, updated only at startup/state changes. Entry and exit animations each pass 72 lower/upper barrier cases: closure preserves map/world position, layer, sector, receiver and animation cursor, and reopening after 20/120 ticks completes traversal. Opposing-PC ladder traffic passes 10 scenarios / 24 traversals, including independent simultaneous use of copied ladders; see [occupancy evidence](map-compilation-evidence.md#opposing-traffic-on-exported-ladders-2026-10-09). Actor overlap/crushing, group-click formations and broader multi-actor interactions remain open; see [exit-phase evidence](map-compilation-evidence.md#passage-barriers-during-climb-exit-animations-2026-10-09). Historical physical-navigation route counts do not certify the current architecture. Broader placements and rendered traversal remain open. |
| Jump zones and paired edges | Derive from marked surfaces or transform authored edges; find current destinations and trim blocked approaches/flights. | Moved/cross-asset destinations, skills and nearby state changes tested. Rotated/copied roof fixtures pass 120 isolated upright jumps, 120 upright walk–jump–walk sequences and 120 isolated sword jumps with exact landings and preserved combat state. Another 120 interior-click routes pass normal player dispatch with exact arrival; the checked-in fixture verifies overlay selection and missing-skill rejection. Shoulder jumps and assisted player clicks each pass 120 cases including helper recovery. One tested rotation omits unusable receiving spans. Authored vertical pairs pass 160 isolated assisted-ascent/upright-descent cases and 140 player-click routes; 20 occluded clicks are verified separately. Repositioned automatic flat-receiver climbs pass 160 isolated traversals and 160 clicks across four rotations/two elevations; 7,840 native positions are covered by clearance and 32 disconnected-copy checks pass. Surface-only climbing rules independently pass the same 320 traversal/click cases, 7,840 clearance samples and 32 disconnected-copy controls without authored jump records. Sloped receivers add 960 native traversal/click checks and 23,540 covered clearance samples, including surface-derived connections. Mixed-posture sloped climbs add 1,920 native traversal/click checks and 38,240 covered positions. Three-PC group clicks additionally pass 48 directed roof routes / 144 jumps, including crouched landings and separate endpoint footprints. Full combat interactions, assisted group formations, in-flight separation, rendering and broader authoring/traversal coverage remain open. |
| Surface materials | Transform local material regions and rebuild ground/obstacle/receiver links. | Compiler/native lookup fixtures and published definitions exist; complete geometry coverage remains open. |
| Lighting and shadow regions | Transform local contours and bind to current receiving planes/layers. | Published definitions and focused native queries tested; complete placement/query coverage remains open. |
| Environmental sounds | Transform local emitters; preserve timing, falloff, acoustic altitude and ambience filters. Cropped fragments share one clock using the [sound shape extension](sound-source-format.md). | Native construction tested across exported maps. Disconnected fragment export, moved/curved/rising placement, local recovery, native distance/noise queries and invalid-index rejection pass; audible playback and ownership review remain open. |
| Animated scenery | Transform local billboard anchors/boundaries; package pinned profiles and frames. | Authoring, publication, previews, normal native preload and ZIP construction tested. Six-frame GPU fixture passes Vulkan/OpenGL; scene ordering, fog, masks and shadows remain open. |
| Interactive state changes | Compile local initial/applied collision, sight, masks, appearance and door permissions into fresh control bindings. | Native apply/reset and paired bake fixtures pass. Complete asset/visual coverage remains open. Best-effort mode retains excess controls' initial states at the 16-switch-per-area runtime limit. |
| Map settings | Use editor identity/bounds and asset-owned forest/material defaults. | Compiler/runtime tests pass; ambience selection remains mission-owned. |
| Resource banks and references | Package baked images and pinned asset resources; resolve shared installed resources. | Independent scenery ZIP loading tested. Unpinned sprite/audio references still require the base installation. |
| Reopenable editor document | Include the original editable level JSON and pinned asset references in the ZIP. | Save/reopen/export regressions pass. |

Automatic jump matching now deduplicates identical bidirectional connections
regardless of ledge discovery order. Regression coverage checks all six orders
of two edges plus a duplicate, while preserving separate landing-zone identities
and different clearance policies. The focused compiler/jump suite passes 163
tests; this addresses duplicate connections, not the remaining actor-flight gaps.

Spline clearances can now preserve distinct physical and navigation planes using
per-vertex navigation heights. Tests cover rising curved spans, holes, frame
translation, malformed heights and nonplanar rejection. Tilted source frames that
separate the two footprints still warn/omit the clearance. The 183 focused compiler,
spline, placement and translation tests pass. Native construction and 96 thin
movement probes now pass across repeated openings on flat/rising terrain at two
elevations, with clearances enabled/disabled and solid pillars preserved. This
exposed and fixed a rounding gap under nominally touching sloped walls: contact
and clearance-plane matching now tolerate 1/1024 map unit, while a tested real
0.01-unit gap remains open. A widened-opening follow-up passes 48 directed
actor-sized pathfinder routes in both directions, with a maximum route length
that rejects detours around the wall. Complete sprite traversal now passes 48
directed opening routes across three repetitions, flat/rising terrain and two
elevations. The audit dispatches native movement, checks collision and receiver
height at every tick, bounds travelled distance, and waits for the stop animation
and order queue to finish. A zero-distance stop's reserved 0.01-unit offset is
accepted only after that specific animation completes.

Rotated openings now pass the expanded `--rotated` batch from
`refinement/check-spline-clearance-planes.mjs`: 32 exports at 0/37/90/180 degrees,
two elevations, flat/rising terrain and enabled/disabled clearances pass 384
thin movement probes and 192 bounded pathfinder routes. All 192 complete sprite
crossings pass, with maximum travelled distance 39.991 for a 40-unit crossing
(limit 45). Every tick retains collision clearance and the expected receiver.

This exposed a compiler error: clearance contours and holes used physical
height for projection while their plane used navigation height. They now use
navigation coordinates consistently, preserving pillars in their authored
position. Ordinary rotated-clearance regression coverage also checks that a
whole-footprint cut leaves no shifted obstruction and a preserved hole still
blocks navigation. The synthetic ground gradient was separately corrected to
stay constant across spline cross-sections in scene space; the oblique camera
means a game-coordinate perpendicular is insufficient. All 287 selected
compiler/export/spline tests pass. Curved flat-ground openings now additionally
pass 16 exports at the same four rotations and two elevations: 192 collision
probes, 96 bounded graph routes and 96 complete sprite crossings preserve the
openings and intervening pillars. Maximum travelled distance is 39.990033 units
for a 40-unit crossing. Use `--rotated --curved` with the same verifier; see the
[curved traversal evidence](map-compilation-evidence.md#curved-spline-opening-traversal-2026-10-11).
The `--curved --deck` audit exposed a blocked opening where independently
triangulated clearance and floor planes failed to match. Export now maps matching
source clearances through the receiving triangles. All 192 collision probes and
96 bounded graph routes pass across four rotations/two heights. All 96 complete
sprite crossings also pass with per-tick collision, receiver and height checks;
maximum travelled distance is 39.990021 units for a 40-unit crossing.
All 16 exports also pass editable ZIP round-tripping with `--zip-roundtrip`:
geometry, warnings, scene and pins survive reopening/recompilation, and the
ZIP-extracted descriptors pass the native collision and graph-route checks.
Diagnostic blank rasters do not establish rendered appearance.
The `--copied` extension places two independent spline paths in the same map.
Across four rotations/two heights, all 16 editable ZIP round trips, 384 collision
probes, 192 bounded routes and 192 complete sprite crossings pass. Routes retain
their own sector and receiver bindings; maximum travelled distance is 39.990029
units for a 40-unit crossing.
Explicit upright headroom retains the uncut wall and authored pillar holes.
Curved terrain conformance and rendered actor integration remain open; see the
curved traversal evidence above.
Use the ignored native `exported_openings_support_complete_sprite_routes` test
with `ROBIN_ASSET_MAP_DIAGNOSTICS` set to the generated directory and
`ROBIN_CLIMB_RHS` set to a complete character sprite to reproduce the actor audit.
Pipeline typechecking remains blocked
by existing errors in `state-delivery.test.ts`; touched files pass scoped lint and
app typechecking/build.

Mission content remains separate from reusable map definitions:

| Mission information | Current construction / status |
|---|---|
| Player starts | Explicit Mission-tab entries export as `spawn_points`; resolve current terrain, sector/layer and receiver. No implicit PC is added to an empty map. |
| NPC soldiers | Explicit entries export as `soldiers`, retaining type, facing and allegiance and resolving placement against compiled geometry. |
| Imported missions | Explicit import restores available PC slots and soldiers with warnings. Preview population is not silently exported. |
| Scripts, patrols, inventory, objectives and other actors | Not provided by the minimal mission editor; outside map-only compilation parity. |

Detailed results, reproduction commands and historical diagnostics are in
[map compilation evidence](map-compilation-evidence.md) and
[testing instructions](../../docs/TESTING.md). Recovered record counts measure
asset-authoring progress; they are not a compiler acceptance criterion.
