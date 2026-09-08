# 3D level editor

The editor (`app/`) works on the volume reconstruction of a map
(`pnpm volumes --map <map>` → `library/scenes/<map>-volumes.scene.glb`, one
node per obstacle) together with the game's level data from the hackable
datadir. It keeps a `Level3D` document (`shared/src/level3d.ts`): every
part carries the game's own obstacle (footprint polygon with absolute
`z_bottom`/`z_top` per point, flags) plus an editor transform (translate in
map px and z, turn about the footprint centroid in map coordinates), and
parts are grouped into buildings. A building in the game data is a stack
of obstacles (opaque box to the eave, jettied floors, roof slopes,
chimneys, door posts, furniture); `groupObstacles` joins obstacles whose
footprints overlap by 40 % of the smaller one and whose height ranges
touch (York: 975 obstacles → 459 buildings; the castle keep is one 36-part
group). A building has its own transform, applied after its parts' own.
The document is saved as `library/scenes/<map>.level3d.json`; `pnpm bake`
turns it back into game files.

## Running

```
cd level-editor && pnpm install
pnpm --filter app dev            # http://localhost:5180
```

Open the hackable datadir (read) and the library folder (read/write; it
holds `scenes/`). Pick a map in the bar. If no `<map>.level3d.json` exists
the document is built from the reconstruction and the level.

## Controls

| action | how |
|---|---|
| pan / orbit around the point under the cursor / zoom to cursor | left drag / right drag / wheel |
| game camera (the map's own view) | `g` or the button |
| frame everything | `f` |
| select building / single part | click / alt-click (or click again inside the selected building); `Esc` clears |
| move | drag the selected building/part along the ground, or the gizmo (tick "lift" for height), or type dx/dy/dz |
| turn | `q` / `e` (15°) or type rot_deg |
| duplicate / delete | `d` / `Del` |
| hide | checkbox (hidden buildings and parts are left out of the bake) |
| snap a floating part | parts tagged "float?" show the suggested Δ; the button shifts y and z by −Δ (same map pixels) |
| undo / redo | `ctrl+z` / `ctrl+shift+z` |
| save | `ctrl+s` |
| overlays | obstacle outlines (document state), elevation lines |

Turning happens in game coordinates, where footprints are rectangles; in
the scene frame (Y stretched by 1/sin elevation) that is an affine map, so
each object is a translation wrapper (the gizmo's target) around a node
carrying the affine matrix.

## Bake

```
pnpm bake --map york [--doc library/scenes/york.level3d.json] [--out work/york-bake] [--fill proc|synth]
```

Reconstructs the map like `volumes.ts` (textures always come from the
original positions), places every object with its transform (duplicates
share the source's faces), renders the map from the game camera with the
software rasterizer at full resolution and writes
`<out>/Data/Levels/Day/<map>.map.png`, `<map>.min.png` (original minimap
size) and `<map>.rhp.json` with the objects' obstacles moved (deleted ones
become empty polygons so indices stay valid, duplicates are appended).
Copy those over a hackable datadir to play. Without a document the output
is the round-trip test: York comes back with 97.7 % of the pixels identical
to the patched map (mean abs diff 3.2 of 765); the rest are 1-px seams at
face boundaries and grazing faces.

Not yet (see `3d-editor-plan.md`): masks are carried through unchanged (a
moved building keeps its old masks), patch states / interiors, Night and
Fog, other ambiances, in-browser bake, footprint editing, mission entities.
