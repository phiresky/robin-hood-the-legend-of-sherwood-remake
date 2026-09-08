# Sherwood Blender reconstruction

Native scene: `level-editor/work/sherwood-refinement/sherwood-refinement.blend`.
All textures are packed. The hidden baseline and original working objects remain
available; replacements have numbered collections. Game collision, source
obstacles and the editor's library GLB are unchanged.

This reconstruction uses the single Day map and authored animated overlays.
Visible paths, footprints and heights follow those references. Hidden thickness,
cross-sections, canopy depth and rear materials are inferred.

## Inspection

In **Sherwood Refinement**, frames 1–9 select the reference camera, east orbit,
west orbit, plan, village/bridges, central oak, camp, riverbank and ladder-oak
orbit. Set the frame before rendering: camera markers override a camera assigned
while another marker's frame is active. Numpad 0 enters the camera. Camera Data >
Background Images contains the bare map and composite overlays, disabled
initially. Enable either at 50% to check registration.

**Sherwood Animation Reference** uses an independent 1–64 timeline at 25 fps.
Six tree canopies retain all sixteen authored frames and changing offsets,
advancing every four ticks. They remain separate from the trunks. Original cards
are preserved but hidden. Fourteen water, butterfly and fire references use
their first frame; this is not a full runtime FX preview.

Inspection outputs in `pass2/`:
- `reference-comparison.png`: source composite and Blender, with close-ups.
- `refined-structure-orbit.png`: solid geometry with foliage/FX hidden.
- `refined-foliage-orbit.png`: textured oblique view.
- `refined-river-close.png`: riverbank inspection.
- `animation-01.png` and `animation-33.png`: moving foliage check.

`inspection-depth/` contains thirteen additional solid, triangulated wireframe
and textured renders, including foliage-hidden structure views. Generate these
with `render_inspection.py` through MCP. The wire renders hide the clearing mesh
so its dense grid does not obscure the structures.

Canopy depth follows supporting trunk footprints instead of the sprite's
top-left/elevation billboard plane. The current geometry uses 470 separate,
irregular rounded clumps distributed through crown depth. This replaces the
warped mask shells and their long connecting walls. Broad crowns interpolate
between supporting trunks; the foreground Arbre05 root remains inferred.
The original animated image is projected onto these volumes, with explicit
canvas clipping to prevent neighboring atlas frames bleeding into the borders.
New images are named `inspection-depth/rounded-foliage-*.png`; the corresponding
topology report is `rounded-foliage-validation.json`. The prior native scene is
preserved as `sherwood-before-rounded-foliage.blend`.

## Geometry passes

| Collection | Reconstruction |
| --- | --- |
| 03–06 | Long suspension bridge, ladder oak, three radial platforms, furniture supports |
| 08 | 22 tapered/fluted trunks, root buttresses and traced branches |
| 09 | Central oak fork; plank-built hut, peaked roof, porch rails and ladders |
| 10 | Six crowns containing 470 closed foliage clumps with animated alpha atlases |
| 11 | Separate authored ambient overlay references |
| 12 | 61 boards across two bridges/two landings, beams and rope rails |
| 13 | 26 faceted boulders, river-fence posts/rails and fallen branches |
| 14 | 1,375 wall boards and roof shingles across huts and treehouses |
| 15 | Subdivided river bluff and shallow clearing relief; filled ground texture retained |
| 16 | Cooperage, supply barrel, hollow cauldron, stools, spit and traced roots |
| 17 | Remaining ladder, upper oak rungs, concealed bark and border-hut timber UVs |

The central oak's two overlapping trunk obstacles are replaced together. Its
main trunk ends at the fork instead of extending through the hut. Root valleys
break up the circular flare. Zero-thickness ladder obstacles 97, 98 and 101
were absent from the GLB; their visible details are modeled explicitly.

The RHP has zero `patches` and twenty `animations`. Trees Arbre01–06 come from
`shertree.rhs.d`. Sprite top-left is position plus frame offset; elevation enters
the 3D anchor but cancels in projection. Extraction removes transparency key
RGB (0,251,0) and retains source frame metadata.

## Reproduction

Source assets must exist in `datadirs/fullgame_gog_hackable/Data`, or set
`SHERWOOD_DATA_DIR` before launching Blender/Python. Only the game datadir may
come from the main repository; worktree inputs and outputs remain local.

Run `extract_animation_references.py` with normal Python, Pillow and NumPy.
It creates atlases, the source composite, metadata and the clean bark crop.
The original GLB must exist at
`level-editor/library/scenes/sherwood-volumes.scene.glb`.

Run these scripts **inside Blender through MCP**, in order:

1. `setup_scene.py`
2. `refine_bridge.py`, `refine_oak.py`, `refine_platforms.py`, `refine_camp.py`
3. `import_animation_references.py`
4. `refine_forest.py`, `refine_treehouse.py`, `refine_canopies.py`
5. `refine_walkways.py`, `refine_riverbank.py`, `refine_buildings.py`
6. `refine_terrain.py`, `refine_props.py`, `refine_ladders.py`
7. `refine_sampling.py`, then `validate_refinement.py`

Supply the absolute script path as `__file__`:

```python
scope = {"__file__": path, "__name__": "__main__"}
exec(compile(open(path).read(), path, "exec"), scope)
result = scope["result"]
```

Scripts reject duplicate collections. When revising a pass, inspect and remove
only its generated objects before rerunning; preserve source objects.
The validator renders comparisons, checks topology/drivers, packs textures and
saves the native file. Run `compare_reference.py` with normal Python afterward.

The MCP viewport screenshot returned black and its render helper used an
inaccessible temporary path. `bpy.ops.render.render` through MCP to an explicit
worktree path works. All geometry operations used Blender MCP.

## Validation and practical limits

Before the rounded foliage pass, checks covered 2,287 generated meshes,
134,029 vertices and 128,088 faces.
The 2,286 closed meshes have no non-manifold edges or zero-area faces. Terrain
is one intentionally open surface with a clean boundary. All twelve atlas
drivers are valid; all used file textures are packed. Reports are
`pass2/geometry-validation.json` and `pass2/image-comparison.json`.

Pixel error measures registration, not percent recovered geometry. The original
camera is the reliable reference. Oblique views expose some projected baked
shadows, inferred rear surfaces and the source image's cropped border.
Nearest-pixel texture sampling preserves the source's painted detail. Whole-map
RGB error drops from the untouched baseline's 10.67 to approximately 8.93 on a
0–255 scale. Baseline materials are preserved separately.

TODOs for further art work:

- Individually trace irregular board ends, roof breakage, lashings and joints.
- Give foliage individual leaf clusters and hidden branch networks; current
  clumps retain the source alpha but still infer concealed branch structure.
- Finish small bushes, baskets, utensils, chimney masonry and untouched
  background obstacle shapes.
- Further fit the ladder oak and camp outlines against pixel residuals.
- Improve filled ground and concealed material transitions; separate baked
  lighting where useful.
- Map multipart meshes deliberately to editor obstacle nodes before exporting
  a replacement GLB. The native Blender scene is the deliverable of these passes.
