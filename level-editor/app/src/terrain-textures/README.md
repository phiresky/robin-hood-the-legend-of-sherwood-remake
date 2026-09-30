# Default terrain art

Seamless 1024 × 1024 tiles synthesized from clean Day map surface samples with
`texture-synthesis 0.8.3 --tiling`. Grass and dirt use Leicester; water uses the blue-green Nottingham pond and paving
uses Nottingham courtyard flagstones. The donor crops are stretched vertically
to undo the ground's 35° projection before synthesis. Grass donor lighting is flattened to suppress repeated dark patches. Water receives
a cool color grade to retain visible blue-green reflections.

Regenerate from `level-editor/` with Pillow and `texture-synthesis` on PATH:

```sh
python3 scripts/synthesize-terrain.py /path/to/Data/Levels/Day
```

The script records crop rectangles, seeds and CLI settings. Its working donors
and full-color outputs go to ignored `work/terrain-textures/`. Use `--threads 1`
for deterministic regeneration; parallel synthesis varies in small details.

The PNGs are reviewable swatches. `tiles.json` holds the same 256-color palettes
and zlib-compressed indexed pixels for synchronous browser/offline decoding. This avoids an
image-loading race when baking immediately after adding terrain. Each tile
covers 1024 game units, so moving or resizing ground preserves its texture scale.
Roads and rivers use 256-pixel-wide strips of the dirt/water swatches with feathered
edges and the same 1024-unit length repeat, rather than squeezing the larger tiles
into each short spline repeat.

## Material library previews

`material-previews.png` is a small atlas used by material library cards. It contains
128 × 128 downsampled previews of every named material, followed by four untinted
base tiles for custom material colors. The adjacent JSON records the tile order;
indices run left to right, then top to bottom. Preview generation uses the same
full-resolution designs as terrain rendering and export, then applies a box filter.

Regenerate after changing the material catalog, base tiles, or procedural details:

```sh
pnpm --dir level-editor --filter app generate-material-previews
```

The generated PNG and JSON are committed so opening the library does not generate
full-size terrain textures on the browser's main thread.
