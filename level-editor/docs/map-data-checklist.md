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
Physical-plane conversion and distance-bounded stepping now have native unit
coverage, including stationary screen positions. Physical collision routing also
passes native unit checks for obstacles, rotations and landing support using the
existing pathfinder. Runtime traversal is not yet wired to these APIs, so the
failed routes remain unresolved.
The sprite motion API and ground-coordinate receiver entry now also support
edge-on planes, with animation-distance and position-state round-trip coverage.
Compiler metadata and actor order dispatch still need integration.
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

Latest validation (2026-10-05): 745 editor tests passed, two skipped; the game
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
