# Plan: replace the 2D level editor with a 3D one

Status: proposal, 2026-09-03; first implementation the same day, see `3d-editor.md` (viewer, selection, move/turn/duplicate/delete/hide, undo, save, `pnpm bake` round-trip at 97.7 % pixel identity). The 2D editor (`app/`, modes inspect /
compose / scene) composes cut-out sprites onto a painted map; that track is
dead. The volumes track (`docs/3d-reconstruction.md`) shows the real
structure of a level: the game's own sight-obstacle prisms, textured by
reverse projection, reproduce the map exactly from the game camera. That
makes a 3D scene the natural source of truth and the game's 2D level a
bake of it.

## What the game consumes (hackable datadir, per level)

| file | content | 3D source |
|---|---|---|
| `Levels/<Ambiance>/<map>.map.png` | the painted map | render of the scene from the map camera (elevation fitted per map, `map-camera.ts`); Night/Fog variants by a colour transform learned from the originals |
| `Levels/<Ambiance>/<map>.min.png` | minimap | downscale of the above |
| `<map>.rhp.json` (ProtoLevel) | sight obstacles, masks (RLE per layer + character/projectile polylines + obstacle indices), patches (element FX), material/light sectors, elevation lines, sound sources, jump zones/lines, lifts, motion data | sight obstacles = the volumes themselves; masks = the per-obstacle id buffer we already rasterize (`rasterOwners`); elevation lines = terrace polygon edges; motion obstacles = footprints at ground level; the rest stays authored data carried through |
| `<mission>.rhm.json` | units, civilians, paths, targets, bonuses, scrolls, script objects, tactic data | placed in the 3D scene as gizmos, exported unchanged in structure |
| `<mission>.scb.json` | script | untouched |

Nothing in the game changes: the editor writes the same files the
converter produces. The Rust side only needs to keep loading hackable
levels (`level_loading_host.rs` already does).

## Scene document (source of truth)

`<map>.level3d.json` next to the level plus a GLB with one node per
obstacle (the export `volumes.ts` now writes: `map` → `ground`,
`buildings/building-NNN`, `terraces/terrace-NNN`, shared atlas material).
Per object: the footprint polygon with `z_bottom`/`z_top` per point (the
game's own representation, edited directly), the face tiles, `opaque`,
material/light sector membership, and the source (`york#655` or a library
prefab id). Ground: the terrain as a plateau set (terrace polygons with
height) over a flat base, plus a ground texture. Mission entities as
typed placements with their `.rhm.json` fields.

## Editor architecture

- Keep the pnpm workspace, Solid 2.0 for UI, the datadir/File System Access
  layer (`fs.ts`, `datadir.ts`), the shared schemas, and the pipeline
  (`volumes.ts`, `render.ts`, `map-camera.ts`, texture fill).
- Viewport: three.js (already in the app), WebGL2, unlit textured
  materials, two cameras: the game camera (orthographic, fitted elevation,
  yaw 0, toggled with one key so the bake preview is always one key away)
  and a free orbit camera. GPU id pass for picking (obstacle + face).
- Drop: compose mode, MapDraft, wall stitching, sprite library, SAM 2D
  extraction (`detect.ts`, `extract.ts`, `sam.ts`, `clip.ts`,
  `contact-sheet.ts`, `sweep.ts`, wall tools). Keep the terrain swatch idea
  only as a ground-painting brush.
- Bake runs in the browser (offscreen WebGL render at map resolution, id
  pass for masks) and, identically, in Node (`render.ts`) for batch and
  tests.

## Milestones

1. **All maps through the volumes pipeline.** Run `volumes.ts` on
   Leicester, Lincoln and the full-game maps; fix what York did not need
   (terrace threshold, camera fit, obstacles with holes, water). Output:
   one `<map>-volumes.scene.glb` per map. Gate: same-view render matches
   the map on every level.
2. **Round-trip bake.** From an unmodified scene write `map.png`, `min.png`
   and `rhp.json` (obstacles, masks from the id buffer, elevation lines,
   motion data carried through) into a hackable datadir and play the level
   in the game. Compare our masks with the original masks pixel by pixel;
   this is where the mask semantics (layers, character vs projectile
   polylines) get reverse-engineered against real data. Gate: the game
   runs the baked York/Leicester indistinguishably from the originals.
3. **Viewer.** Load the GLB hierarchy, both cameras, picking and selection
   outline, per-object hide/isolate, overlays in 3D (elevation lines,
   sectors, mission entities as billboards, sight-obstacle wireframes).
4. **Editing.** Move/rotate/duplicate/delete objects (snap to ground and
   terrace tops), edit footprints and heights with vertex handles, edit
   terrace polygons and heights, undo/redo, save/load the scene document,
   bake button. Moving a building exposes faces the map never showed: they
   already carry synthesized textures (`--fill synth`), and the ground
   under it is already filled.
5. **Prefab library.** Every reconstructed obstacle becomes a self-contained
   textured prefab (GLB + atlas slice + footprint); drag from the library
   into any map, with on-demand texture completion for hidden faces
   (texture-synthesis now, AI inpainting per `ai-texture-completion.md`
   later).
6. **Ground and lighting.** Ground painting with synthesized swatches,
   roads and regions (terrain track ideas), water. Painted shadows are
   baked into the ground texture, so a moved building leaves its shadow:
   estimate the sun from existing shadows, remove them from the ground
   (inpainting) and re-render drop shadows at bake time. Night/Fog as
   per-map colour transforms fitted to the original variants.
7. **Mission editing.** Units, paths, targets, triggers, tactic data with
   proper gizmos; `.rhm.json` export.

## Risks and open questions

- Mask semantics are only partly understood; milestone 2 settles them with
  real comparisons before any editing work depends on them.
- The volumes are coarse (no chimneys, dormers, overhangs); fine from the
  game camera, visible from the orbit camera. Acceptable for an editor
  view; hero buildings can get SAM 3D / Tripo meshes later as an option.
- Shadows and painted ground detail are the main obstacle to moving
  things around convincingly (milestone 6).
- Browser bake of a 3136×2318 map with a 16k atlas needs care (tiled
  rendering); the Node path exists as fallback.
- Other maps may break assumptions York met (terrace detection by area,
  absolute heights, single camera elevation).
