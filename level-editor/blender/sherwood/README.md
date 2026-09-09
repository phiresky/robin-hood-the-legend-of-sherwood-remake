# Sherwood Blender refinement — checkpoint 1

Native working file: `level-editor/work/sherwood-refinement/sherwood-refinement.blend`.
The original library GLB and game obstacle data are unchanged. The native file
contains a hidden baseline collection and an independent editable working copy.
Textures and the source Day map are packed into the Blender file.

## Inspection

Nine cameras are bound to timeline frames 1–9: original 35° map view, east,
west, overhead, village/bridges, central oak/hut, camp, riverbank, ladder-oak
orbit. Use the numbered frame and Numpad 0 to inspect that camera. The original
camera has a 50% source-image overlay under Camera Data > Background Images;
enable it to compare in the viewport. It is deliberately absent from renders.
Use Solid shading to assess geometry, Material Preview to inspect projection.

Switch the scene selector to **Sherwood Animation Reference** for the fuller
map with its separate tree overlays. Play frames 1–64 at 25 fps: six authored
trees retain their 16 frames and per-frame offsets, with four ticks per frame
(the stored delay is 3; the engine advances when its counter exceeds delay).
This timeline is independent of the geometry scene's camera bookmarks.
The six trees are camera-facing **reference cards, not reconstructed 3D trees**.
They are excluded from the main geometry inspection view layer. Ten river
overlays, fire and three butterflies are retained as first-frame references.
Runtime phase, draw sorting and shadow rules are not reproduced by this preview.

The source RHP has zero `patches` but twenty `animations`. The six tree profiles
are `Sherwood - Arbre01` through `Arbre06` in `shertree.rhs.d`. Placement follows
`position + frame offset`; elevation participates in the 3D anchor and cancels
from the projected pixel position. Extraction removes the converted RGB565
transparency key `(0,251,0)` and retains the original frame paths and metadata.
The source plus all twenty first frames is available as
`animation-references/sherwood-composite-frame00.png`; the tree contact sheet
is beside it. Use this composite as an additional overlay, leaving the bare
Day map available to inspect geometry beneath the moving crowns.

Saved inspection PNGs and image-error measurements are beside the blend file.
`04-refined-orbit-solid.png` exposes the new geometry;
`04-refined-map.png` checks original-camera appearance.

## Changes made through Blender MCP

- Obstacle 94: 39 closed deck planks, traced sagging hand ropes, six posts,
  two lower supporting ropes. The old slab is hidden in the working copy.
- Obstacle 24: tapered, fluted round trunk, a root flare, two rope ladder
  stiles and 21 rungs. Trunk geometry extends above the image crop, avoiding
  a visible cap at the artificial sight-obstacle height.
- Obstacles 86, 88, 89: 161 closed radial boards, retaining the existing
  platform outlines, heights and access cutouts.
- Camp obstacles 11, 12, 14–20: 36 legs and four stretchers beneath the
  existing furniture slabs.

Each pass has its own numbered collection. Replaced working objects retain
their `source_obstacle` and `replaced_by` properties. The full baseline remains
available for comparison. The refinement scripts preserve source attribution
and tag inferred geometry.

## Reproduction

These scripts run **inside Blender**, through the MCP execute-code tool. In a
fresh Blender session, execute `setup_scene.py`, then `refine_bridge.py`,
`refine_oak.py`, `refine_platforms.py`, and `refine_camp.py` in that order. Give
each script a namespace with its absolute `__file__`:

```python
scope = {"__file__": path, "__name__": "__main__"}
exec(compile(open(path).read(), path, "exec"), scope)
result = scope["result"]
```

Save the blend after the passes. Scripts reject duplicate collections so an
accidental rerun does not stack geometry. The interactive MCP viewport capture
returned black images in this session, and its render helper returned a
non-accessible temporary path. `bpy.ops.render.render(write_still=True)` via
MCP produced accessible inspection renders successfully.

For the animated references, first run `extract_animation_references.py` with
normal Python (Pillow and NumPy), then execute `import_animation_references.py`
through MCP with the same namespace convention. This adds the separate
animation inspection scene and packs all six frame atlases into the blend.

## Validation and remaining work

The new geometry is checked for manifold edges and degenerate faces. Original
camera renders are compared with the original PNG as a registration check.
Pixel agreement **does not measure 3D fidelity**: much of the unmodified scene
looks correct only because its detail is painted into a projection texture.

Checkpoint checks: all 274 new closed meshes (11,988 vertices) passed manifold
edge and zero-area-face checks. All twelve tree-atlas drivers are valid;
renders at ticks 1 and 33 differ, confirming the sequence advances. Original
camera RGB error improves slightly overall (10.666 to 10.645 on a 0–255 scale),
but the ladder-oak crop regresses (11.734 to 12.084). TODO: further fit its
outline and platform junction; the rounder geometry is not yet a better pixel
match everywhere. These measurements use the bare Day map; the animation
composite is a separate reference and must not be mixed into that comparison.

This is an initial refinement, not a 100%-fidelity reconstruction. TODO:

1. Trace the bridge's missing/broken boards, individual board widths, rope
   ties and hanging pieces; current divisions are approximate.
2. Refine individual platform board ends and underside brackets. Split the
   other bridges, landings, hut walls and roofs into their actual construction.
3. Reconstruct remaining square trunks, branching, buttress roots and foliage.
   The ladder oak's hidden half, cross-section depth and fluting are inferred;
   rear bark currently borrows front texture and includes painted ladder detail.
4. Replace the largely flat riverbank with terrain, individual rocks, water
   surfaces, fallen timber and vegetation. Current obstacle heights alone do
   not describe these forms.
5. Trace furniture supports individually; the current four-leg arrangements
   are inferred. Model canvas, trestle joinery, utensils, baskets and barrels.
6. Remove projection ghosts from the ground and separate baked lighting from
   material colour before claiming reliable views around the whole scene.
7. Recover original 3D assets or additional views if exact concealed geometry
   is required. A single rendered map cannot establish it uniquely.
8. Plan the level-editor export: new multipart details need deliberate mapping
   to original obstacle nodes. This checkpoint is a native Blender document,
   not a replacement editor GLB or a modification of gameplay collision.
