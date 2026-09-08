# Terrain track — design note

The litmus test (`work/leicester-comparison.png`) shows the ground layer is the
biggest gap: grass, dirt roads, moat/river water, forest edges. None of this is
extractable as discrete objects; the plan is compose-time synthesis from
texture swatches plus vector features.

## Asset side (exists)

`scale_class: "texture"` assets — rectangular swatches cut via `mode: "crop"`
proposals (no SAM): grass, dirt road, moat water, forest canopy, ploughed
field. Multiple swatches per material welcome (variation).

## Compose side (to build, in this order)

1. **Ground fill**: per-map base color + tiled swatch fill with random offset /
   rotation-free wang-ish blending (large soft-brush alpha between two swatch
   layers hides tiling; the originals are hand-drawn so perfect tiling is not
   the goal, "no visible seams at game zoom" is).
2. **Terrain regions**: painted polygons assigned a material (grass, dirt,
   water, field). Rendered by clipping the swatch fill to the polygon with a
   feathered edge (~8-16px). Water gets a darkened edge band (the originals
   shade banks darker).
3. **Roads**: splines with width; rendered as a dirt-material stroke along the
   path, feathered, with slight width jitter. Same machinery as walls
   (spline-segment stitching) but filled with texture instead of sprite
   segments.
4. **Scatter**: bushes/rocks/tufts stamped along region borders (forest edge =
   canopy swatch region + tree/bush scatter on the boundary).

MapDraft gains `terrain` later: `{ base_material, regions: [{material,
polygon, feather}], roads: [{spline, width, material}] }`. Baking to a final
map PNG happens at export, not per-frame — the editor canvas can render a
cheaper preview.

## Known converter gap (Rust todo)

Sprite-bank PNGs exported by `convert_datadir` keep the RGB565 transparency
key as opaque green (0,251,0) instead of alpha; `pipeline/src/fx.ts
loadKeyedFxPng()` works around it. Fix in the converter eventually.
