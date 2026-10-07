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

Landing validation now excludes permanently blocked portions of shared stair
edges on either the flight or landing while retaining their collision.
Switchable blockers and uncovered edge portions still require matching heights.
Focused regressions pass, but the
three-entrance Sherwood candidate still exposes an incompatible unblocked edge;
its traversal remains unresolved. Generated collinear clipping holes no longer
disable physical navigation at 37 degrees. The latest unpublished candidate
emits physical navigation at all eight placements but fails all 48 actor routes,
including the twelve routes previously using projected navigation.

The current library-wide audit (`gameplay-coverage-Bi9ENk`) finds 29 of 1,279
indexed assets without gameplay definitions. One is placed: Croisement01 tree 03.
The timber bridge, both ferns, fallen log, upright fences 94/95 and supplemental
wood 09/44 have published draft definitions. These counts
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
The keep-annex candidate remains unpublished: its saved-map routes pass, but
eight of 32 copied/rotated climb routes fail at 90 degrees. Reliable new-map
placement, rather than matching an existing layout, is the acceptance criterion.
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
The remaining three-entrance Sherwood ladder oak needs a mesh-based platform
and climbing-surface review: its authored platform hole excludes mostly
mesh-supported upper approaches, while its rung centers diverge from the current
climbing plane. The reconstructed plank footprint is still an unpublished
candidate; all three connections must survive the correction.
The first three-entrance candidate retains all doors and passes 36/48 moved
routes, with twelve 90-degree failures. A separate compiler correction accepts
ladder seams on platform-hole edges while still rejecting hole interiors.
The focused native fixture passes both directions; a broader rotation check
exposed a receiver mismatch also present without a hole. That mismatch is now
fixed: ladder exit animations retain the physical seam instead of snapping back
to its integer waypoint, and all eight rotated fixture routes pass. Neither
complex Sherwood candidate is published, and its traversal remains unresolved.
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
batch reopened with gameplay definitions for their placed assets; the newer
library audit above identifies four uncovered placements in Croisement03.
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
