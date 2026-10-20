# Wychford — The Tollkeeper's Ledger

An editable fortified market town assembled from Leicester, Derby and Sherwood
assets. The village occupies the west bank; the stronghold stands on the ridge
across the river to its east. Lincoln assets are not used.

## Layout

The market bridge connects the western church, shops and working yards to the
stronghold gate. Smaller lanes connect cottages, gardens and the mill crossing.
The eastern pasture contains the stable yard and gatekeeper's lodging. Fences
mark gardens, church boundaries and paddocks rather than blocking the roads.
Wooded margins, bank meshes and distinct Sherwood boulders break up the terrain.

`layout.json` contains the building, prop, fence, terrain and main-lane placements.
The published scene contains 164 placements and 96 asset source references,
35 editable road/paving splines, one river and one curtain wall. Fifteen named
lanes come from the layout; eighteen doorstep lanes and two courtyard approaches
are recovered from `terrain-layout.svg`.

The curtain uses Derby's wall segment and matching conical turret. Its irregular
perimeter has straight curtain spans, a width of 44 units, and uniformly scaled
2× towers with a 0.5 horizontal width multiplier. Sharp turns
insert towers automatically and terminate adjacent wall spans; gentler controls
retain a continuous curved wall. The gate opening remains deliberate. The wall's
parapet faces outward. Tower placement, minimum turn angle, size, orientation and
individual corner opt-outs remain editable in the Paths panel. A named wall preset
can be saved for reuse across levels in the same browser. Presets are browser
preferences; a map saves its own settings and pinned model references.

The four selected tree models are Leicester's irregular moat-bank and southeast
cottage trees plus Sherwood's leaning and spreading forms. The export helper
freezes Sherwood's animated leaf atlas into a static cutout texture with explicit
UVs and alpha for editor rendering and shadows. Other staged tree forms remain
available in the library but are not used in this layout.

## Ground, paths and lighting

The ground is an editable connected mesh with 1,194 vertices and 2,281 triangular
cells. It is adaptively sampled from the published ground model, retaining more
vertices along the riverbanks and steep slopes. Six editable material regions
recover woodland, grass, dry grass, rocky ground, pebbled banks and the bare-earth
courtyard from `terrain-layout.svg`. Roads and paving are separate editable
splines; no road artwork is baked into the new terrain. Every vertex has an
explicit material assignment, with cell materials retained as fallbacks.

`terrain.jpg` remains the 5120 × 5120 visual reference, with its production record
in `terrain-generation.json`. It is deliberately not applied to the editable
mesh, because doing so would show the old roads underneath moved or deleted
splines. Materials approximate the drawing's semantic regions, not every detail
in the generated image.

To reproduce the migration, use the local library containing the map's models
and descriptors, including `library/3d-assets/wychford/wychford-terrain/model.glb`, install NumPy and SciPy,
and run from the repository root:

```sh
python3 level-editor/scripts/migrate-wychford-terrain.py
python3 level-editor/scripts/migrate-wychford-terrain.py --check
```

With SciPy 1.18.1, verification compares 86,281 samples: every source vertex,
triangle centroid and unique edge midpoint. The maximum sampled height error is
1.994 pixels, mean 0.233 and 95th percentile 1.012. These are sampled errors, not
a mathematical bound at every possible position. The original 28,600 triangles
become 2,281. Placements, manual heights, river/wall controls and mission characters are
preserved exactly; this conversion does not resnap elevated objects. The script
checks those invariants and reproduces the saved terrain and recovered roads.
It also refreshes the cottage and watermill descriptor hashes after verifying
that their pinned model files are unchanged.

The gate is rotated 27° to face outward through the curtain opening. The keep
sits farther west within the enclosure, leaving a passage along the eastern wall;
the smithy, keep approach and sentries follow the revised layout.

`path.png` supplies fine earth and gravel for editable footpaths, with a softened,
irregular alpha edge. Dense height samples keep the authored paths on the ground.
`river.png` is the repeating water and bank tile. The main crossing uses the open
East Village Footbridge, with a smaller timber crossing near the mill.

Sun direction and shadow strength save with the map. Wychford starts with a
northwest sun and 78% shadow opacity; buildings, trees and walls cast onto terrain.
Their baked source colors remain unchanged.

## Build and open

Wychford is already published to `library/scenes/Wychford.rhlos-map.json`. Reload
the connected library and select **Wychford**. Edit the published map in the editor. The terrain migration above is a
reproducible one-time conversion; rerunning it replaces later ground and recovered
road edits, so use `--check` when only validating the committed conversion.

The source Blender workspace and published library models are local assets. The
committed recipe, layout and ground materials reproduce the scene with that library.

## Mission characters

The saved map contains 21 guards as ordinary editable Mission soldiers, retaining
their names, positions, facing directions and matching character profiles. They
use the shared library character catalog and the normal gameplay export path.
Edit them through the **Mission** panel; player spawn points can be added there too.

The former preview civilians, beggars, items, patrol routes and dialogue notes
have been removed because the Mission authoring format does not support them.
Guards no longer follow preview patrols. Mission scripts and objectives remain
unauthored.
