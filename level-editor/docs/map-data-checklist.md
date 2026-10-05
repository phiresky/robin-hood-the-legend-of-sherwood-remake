# Map data: editor → game

The goal is **new maps that play well**, assembled from reusable assets and editor
terrain. Existing maps are regression examples, not templates whose original
records must be reproduced. Export reads only the editor document and asset-local
definitions. One-time extraction may restore missing definitions into assets.
Connections and runtime indices are rebuilt after placement, rotation and copying.

**Full parity is not yet verified.** “Tested” below describes specific evidence,
not certification of every library asset or gameplay case. Best-effort exports
warn about omissions and preserve initial states where possible; warnings are
not evidence that an omitted feature works.

The library-wide audit still finds 34 of 1,289 indexed assets without gameplay
definitions, including the composite Derby keep, several Nottingham buildings,
and York market props. None is placed in the ten saved
scenes, but placing one in a new map still omits its gameplay with a warning.
These need individual ownership/geometry review; empty scenery definitions would
be incorrect for buildings and bridges.
An index-based component audit finds that 32 of these assets have every part
represented in newer gameplay-bearing assets, often larger state assemblies.
Their standalone definitions still need restoring with local control dependencies
and coordinate frames; matching parts alone does not make them interchangeable.
The two remaining old ground assets have no component-owner match.
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
A library-wide local-anchor audit finds unsupported anchors in 38 of 53 authored stair
definitions: Derby 0/10, Leicester 4/8, Lincoln 6/6, Nottingham 11/12,
Sherwood 1/1 and York 16/16. All audited floors are planar. This checks local
definitions, not placed connectivity or actual route failures; corrections and
moved/rotated native traversal checks remain necessary across these assets.
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
The isolated ladder passes 12/16 routes: four failures at 180 degrees also occur
in the old published definition. Both visual and rotated-ladder limitations
remain explicit draft warnings and are not certified as parity.
The expanded local-anchor audit also checks ladders and climbable walls:
6/8 ladder definitions and 9/9 wall definitions have unsupported floor anchors.
These are geometric compatibility checks for physical navigation, not proof that
their current projected routes fail. The east-moat ladder's door midpoints lie
0.209 and 0.181 units outside its floor, with landing-height discrepancies of
0.404 and 0.411 units. Its rotated projected corridor failure therefore needs
both independent navigation coordinates and reviewed endpoint seams; enabling
physical stair navigation for climb types alone would not repair it.
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

Latest editor validation (2026-10-06): 837 tests passed, two skipped. Fresh
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
underlying terrain without adding collision or floors. All ten saved scenes
reopen with no placed asset missing a gameplay definition; this does not certify
the completeness of those definitions. Wychford retains an unused great-keep
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
below, the watermill is the only remaining omitted physical receiver in the
uncalibrated Wychford descriptor. The latest baked ZIP predates these changes.

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
| Spline walls | Measure pinned source meshes and deform local surfaces, collision, material contours, lighting, mask coverage and spatial sound emitters with source rotation, trimming, straightening and path placement. | Geometry comparisons pass 24 combinations; Wychford native loading/state checks pass. Material ownership passes moved/repeated, turned, sloped, corner and fractional-origin compiler checks; native ground/obstacle queries pass for three repeated sections. Explicit point/segment probes pass repeated, rising and curved compiler checks and native-fixture equality; native ambience queries pass. Disconnected lighting probes survive clipping independently and match the native ambience fixture. Automatic curved/rising lighting binds from source surfaces and passes native elevated-layer shadow queries. Spatial sound placement and acoustic settings pass native construction. Broader receiver-material, playback and traversal checks remain open. Repeated masks pass native bitmap, character/projectile boundary, altitude and obstacle-isolation checks; curved/cropped coverage passes editor tests. Longitudinal mask probes preserve bends and reject missing or competing receiving layers. A surviving explicit probe also preserves masks whose point anchor was trimmed. Disconnected receiving probes survive trimming without artificial connections and match the native mask fixture. Point-only cropped mask anchors and disconnected application boundaries remain unsupported, as do stateful sources, global sounds and disconnected sound crops; these emit warnings. |
| Navigation graph and fast-find grid | Build fresh graph/spatial structures from compiled geometry. | Native initial/switch-state route checks pass; no source grid or graph bytes are copied. Full actor coverage remains open. |
| Sight/physical obstacles | Transform local shapes, heights and physical/opaque flags. | Native initialization, apply and reset tested; asset ownership coverage remains incomplete. |
| Projection/elevation receivers | Derive height planes and crossing boundaries from placed receiving surfaces. | Fractional seams, slopes, copies and sampled actor crossings tested; complete placement coverage remains open. |
| Doors, gates and locks | Transform endpoints; resolve current neighbours and local initial/alternate permissions. | Compiler/runtime fixtures pass; incomplete assets still warn. |
| Building interiors | Connect entrances in each asset-local room automatically; use editor links or passage sockets between assets. | Separate, joined, moved, rotated and copied assemblies and editable ZIP round trips tested. |
| Stairs, ladders and climbable walls | Assemble local traversal surfaces/sockets and endpoints; derive independent layers and receiving approaches. | Changing stair barriers pass native rotation/copy checks; complete-animation climbing passes 2,166 directed routes. Changing ladder/wall barriers pass 84 actor/state checks covering four rotations, both wall-top door types and a barrier near the entrance, plus native collision/pathfinder checks. Independent copied controls pass 48 further checks; closing a barrier during climbing passes 24 directed checks. Another 72 checks cover reopening during an existing climb: reopening before path failure completes the route; failed requests retain their timeout without automatic retry. Broader placements and rendered traversal remain open. |
| Jump zones and paired edges | Derive from marked surfaces or transform authored edges; find current destinations and trim blocked approaches/flights. | Moved/cross-asset destinations, skills and nearby state changes tested; broader authoring/traversal coverage remains open. |
| Surface materials | Transform local material regions and rebuild ground/obstacle/receiver links. | Compiler/native lookup fixtures and published definitions exist; complete geometry coverage remains open. |
| Lighting and shadow regions | Transform local contours and bind to current receiving planes/layers. | Published definitions and focused native queries tested; complete placement/query coverage remains open. |
| Environmental sounds | Transform local emitters; preserve timing, falloff, acoustic altitude and ambience filters. | Native construction tested across exported maps; audible playback and ownership review remain open. |
| Animated scenery | Transform local billboard anchors/boundaries; package pinned profiles and frames. | Authoring, publication, previews, normal native preload and ZIP construction tested. Six-frame GPU fixture passes Vulkan/OpenGL; scene ordering, fog, masks and shadows remain open. |
| Interactive state changes | Compile local initial/applied collision, sight, masks, appearance and door permissions into fresh control bindings. | Native apply/reset and paired bake fixtures pass. Complete asset/visual coverage remains open. Best-effort mode retains excess controls' initial states at the 16-switch-per-area runtime limit. |
| Map settings | Use editor identity/bounds and asset-owned forest/material defaults. | Compiler/runtime tests pass; ambience selection remains mission-owned. |
| Resource banks and references | Package baked images and pinned asset resources; resolve shared installed resources. | Independent scenery ZIP loading tested. Unpinned sprite/audio references still require the base installation. |
| Reopenable editor document | Include the original editable level JSON and pinned asset references in the ZIP. | Save/reopen/export regressions pass. |

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
