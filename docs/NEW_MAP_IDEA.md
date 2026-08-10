# Can we add a new map to Robin Hood - The Legend of Sherwood?

The original game has nine reusable maps: the five fortified towns of Derby,
Leicester, Lincoln, Nottingham, and York; Sherwood Forest; and three crossroads
maps (`Croisement01`–`03`).

It would be very nice to expand the game with a tenth map. The idea is an
entirely new castle, with a river running past it and a town spread out in
front.

## Maps in the game

These renders come from full-map screenshots (`cargo run --example render_mission_maps`).
Click one to open the full image (full-resolution but lossy, ask me if you need lossless).

| Map | Day | Fog | Night |
| --- | --- | --- | --- |
| **Crossroads 1** (`Croisement01`) | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-1-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-1-day.jpg" width="180" alt="Crossroads 1 by day"></a><br><sub>Tactical mission 1</sub> | — | — |
| **Crossroads 2** (`Croisement02`) | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-2-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-2-day.jpg" width="180" alt="Crossroads 2 by day"></a><br><sub>Tactical mission 2</sub> | — | — |
| **Crossroads 3** (`Croisement03`) | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-3-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-3-day.jpg" width="180" alt="Crossroads 3 by day"></a><br><sub>Tactical mission 3</sub> | — | — |
| **Derby** | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-day.jpg" width="180" alt="Derby by day"></a><br><sub>Attack Derby</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-fog-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-fog.jpg" width="180" alt="Derby in fog"></a><br><sub>The Outlaw and the Prince</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-night-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-night.jpg" width="180" alt="Derby by night"></a><br><sub>Save Tuck</sub> |
| **Leicester** | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-day.jpg" width="180" alt="Leicester by day"></a><br><sub>Contact Ranulph</sub> | — | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-night-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-night.jpg" width="180" alt="Leicester by night"></a><br><sub>Save Scarlett</sub> |
| **Lincoln** | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-day.jpg" width="180" alt="Lincoln by day"></a><br><sub>Robin's Godfather</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-fog-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-fog.jpg" width="180" alt="Lincoln in fog"></a><br><sub>Attack Lincoln</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-night-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-night.jpg" width="180" alt="Lincoln by night"></a><br><sub>Free Godwin</sub> |
| **Nottingham** | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-day.jpg" width="180" alt="Nottingham by day"></a><br><sub>The Sheriff of Nottingham</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-fog-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-fog.jpg" width="180" alt="Nottingham in fog"></a><br><sub>Free Robin</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-night-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-night.jpg" width="180" alt="Nottingham by night"></a><br><sub>Contact Marian</sub> |
| **Sherwood Forest** | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/sherwood-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/sherwood-day.jpg" width="180" alt="Sherwood Forest by day"></a><br><sub>Sherwood Forest</sub> | — | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/sherwood-night-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/sherwood-night.jpg" width="180" alt="Sherwood Forest by night"></a><br><sub>Sherwood Outro</sub> |
| **York** | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-day-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-day.jpg" width="180" alt="York by day"></a><br><sub>Save Marian</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-fog-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-fog.jpg" width="180" alt="York in fog"></a><br><sub>Attack York</sub> | <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-night-full.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-night.jpg" width="180" alt="York by night"></a><br><sub>Lackland's Plan</sub> |

Nottingham also has a `Custom1` background used by *The Silver Arrow*; it is
outside the three standard ambience columns above.

These renders are complete mission scenes. Underneath each scene is a large,
static `.map` bitmap. Mission-specific patch sprites are drawn over it to hide
and reveal building interiors, add doors and other stateful details, and
animate elements such as trees, water, fire, and butterflies.

## Overlay examples

These loops alternate a complete mission render with its raw Day background.
Everything that appears and disappears is supplied by mission-specific entities
and overlays; both frames retain the same map framing and scale.

<table>
  <tr>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-mission-vs-background.avif" width="420" alt="Animated comparison of Contact Ranulph and the raw Leicester Day background"><br><strong>Leicester</strong><br><code>Contact Ranulph.png ↔ Day/Leicester.map.png</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/tactical-4-mission-vs-background.avif" width="420" alt="Animated comparison of Tactical mission 4 and the raw Crossroads 1 Day background"><br><strong>Tactical mission 4</strong><br><code>Tactical mission 4.png ↔ Day/Croisement01.map.png</code></td>
  </tr>
</table>

The examples below are original overlay frames extracted from the retail RHS
profiles. Each animation uses a shared canvas calculated from the profile's
`center` anchor and every frame's individual `offset`; frames have not been
trimmed and independently centered. This preserves the same registration used
by the game renderer. The previews are enlarged to make the smaller mechanisms
legible, but their relative placement is unchanged.

<table>
  <tr>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/sherwood-tree-animation.avif" width="260" alt="Animated Sherwood tree canopy"><br><strong>Sherwood tree canopy</strong><br><code>shertree.rhs: Sherwood - Arbre01</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/sherwood-river.avif" width="260" alt="Animated Sherwood river overlay"><br><strong>Sherwood river</strong><br><code>sherwood.rhs: Sherwood - Riviere_b1</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-tree-animation.avif" width="260" alt="Animated Crossroads 1 tree canopy"><br><strong>Crossroads 1 tree canopy</strong><br><code>Treecr01.rhs: Croisement01 - Arbre01</code></td>
  </tr>
  <tr>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/derby-drawbridge-animation.avif" width="260" alt="Animated Derby drawbridge"><br><strong>Derby drawbridge</strong><br><code>Derpatch.rhs: Derby - Pont_levis01</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-drawbridge-animation.avif" width="260" alt="Animated Leicester drawbridge"><br><strong>Leicester drawbridge</strong><br><code>leipatch.rhs: Leicester - Pontlevis01</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/lincoln-drawbridge-animation.avif" width="260" alt="Animated Lincoln drawbridge"><br><strong>Lincoln drawbridge</strong><br><code>Linpatch.rhs: Lincoln - Pont_levis</code></td>
  </tr>
  <tr>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-portcullis-animation.avif" width="260" alt="Animated Nottingham portcullis"><br><strong>Nottingham portcullis</strong><br><code>notpatch.rhs: Nottingham - herse</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/york-portcullis-animation.avif" width="260" alt="Animated York portcullis"><br><strong>York portcullis</strong><br><code>Yorkpatch.rhs: York - herse</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/leicester-windmill-animation.avif" width="260" alt="Animated Leicester windmill blades"><br><strong>Leicester windmill</strong><br><code>Leifx.rhs: Leicester - moulin</code></td>
  </tr>
  <tr>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/nottingham-windmill-animation.avif" width="260" alt="Animated Nottingham windmill blades"><br><strong>Nottingham windmill</strong><br><code>notfx.rhs: Nottingham - moulin</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-pit-animation.avif" width="260" alt="Animated opening pit trap at Crossroads 1"><br><strong>Crossroads 1 pit</strong><br><code>Trapcr01.rhs: Croisement01 - hole</code></td>
    <td><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/crossroads-trap-animation.avif" width="260" alt="Animated trap overlay at Crossroads 1"><br><strong>Crossroads 1 trap</strong><br><code>Trapcr01.rhs: Croisement01 - piege01e</code></td>
  </tr>
</table>

## Creation of maps

The surviving production art suggests that the original maps were drafted,
built as 3D scenes, rendered with an orthographic camera, and then painted over
in 2D. The result was exported as the static terrain bitmap.

The Leicester making-of sheet shows that progression particularly clearly:

<p align="center">
  <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/making-of-leicester.jpg"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/making-of-leicester.jpg" width="372" alt="Making-of sheet showing Leicester from blockout to painted game map"></a>
  <br><sub>Leicester: 3D construction, render, and painted final map.</sub>
</p>

The original game's concept art also preserves a smaller sketch-to-render
example, showing how a fortified tower design was translated into a finished
environment asset:

<table>
  <tr>
    <td><a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/original-castle-concept-sketch.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/original-castle-concept-sketch.avif" width="360" alt="Original Robin Hood castle tower concept sketch"></a><br><strong>Concept sketch</strong><br><code>castle_1.jpg</code></td>
    <td><a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/original-castle-concept-render.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/original-castle-concept-render.avif" width="360" alt="Finished render of the original Robin Hood castle tower concept"></a><br><strong>Finished render</strong><br><code>castle_2.jpg</code></td>
  </tr>
</table>

The Derby production images are useful as architectural
reference for the proposed castle's massing, towers, gatehouse, and defensive
layers:

<table>
  <tr>
    <td><a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/production-art-derby-blockout.jpg"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/production-art-derby-blockout.jpg" width="360" alt="Original Derby castle blockout"></a><br><strong>Blockout</strong><br>Original 800×1115 artwork.</td>
    <td><a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/production-art-derby-final.jpg"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/production-art-derby-final.jpg" width="360" alt="Original painted Derby castle production art"></a><br><strong>Painted result</strong><br>Original 593×850 artwork.</td>
  </tr>
</table>

In addition, each map needs walkable geometry,
height and sector data, obstacles, interactive objects, patches, and animated sprites.

<p align="center">
  <a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/gameplay-geometry-overlay.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/gameplay-geometry-overlay.avif" width="720" alt="Game map with its gameplay geometry visualized as colored wireframes"></a>
  <br><sub>A game map with its collision, sector, and gameplay geometry overlays visualized.</sub>
</p>



The game itself includes very simplified versions of the 3D models, used to calculate some things like sound occlusion and arrow trajectories: https://www.youtube.com/watch?v=rs7UrwmwqE0



For a new map, the same pipeline is sensible: lock the gameplay layout
first, lay it out in 3D with an orthographic camera, test all routes and
elevations, render the base, paint it to match the original game's style, then author collision, sectors, patches, and ambient animation.

Gameplay needs to be considered: There should be multiple ways into the castle and routes in general, soldiers should be able to be placed and to patrol some routes, etc. The more the designer of the map knows about how the game works, the better. If you have never played the game, here is a gameplay video: https://www.youtube.com/watch?v=ijkwe15y3e4
Note that in-game, you do not see the whole castle at once, you scroll around the map.

## AI attempt: Greywatch Castle

I (a developer, but not an artist) attempted to create a new map with the help
of AI. **Greywatch Castle**: an original, portrait-oriented mission
space with a fortified summit, a river, a mill and bridge hamlet, a church village.

<table>
  <tr>
    <td><a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/greywatch-layout-sketch.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/greywatch-layout-sketch.avif" width="360" alt="Layout sketch for Greywatch Castle"></a><br><strong>Layout sketch</strong></td>
    <td><a href="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/greywatch-castle-concept.avif"><img src="https://raw.githubusercontent.com/phiresky/robin-hood-the-legend-of-sherwood-remake/refs/heads/main/docs/NEW_MAP_IDEA/greywatch-castle-concept.avif" width="360" alt="Best Greywatch Castle AI concept map"></a><br><strong>Best result</strong></td>
  </tr>
</table>

On first glance this looks pretty good and i think it's good for inspiration, but if you look closer it does not work at all. The river flow along the windmill makes no sense, some of the fences and paths just end unexplicably, and so on. Also, the perspective does not work: It's a semi-perspective render, but we need a orthographic / isometric render - the top of the map must have the same scale as the bottom, so that the fixed-size 2D characters can be placed anywhere and have the same relative size.

I tried a few different approaches - starting with a layout SVG or a sketch, generating the image in tiles (this map is too detailed to generate one shot at full resolution), etc, but it's clear that it's not possible right now to create a new map for this game with only low-effort AI.
