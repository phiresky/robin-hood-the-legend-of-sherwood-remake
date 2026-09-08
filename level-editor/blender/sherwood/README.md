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
Frame 10 is a close oblique view of the foreground tree's leaf and branch geometry.

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
top-left/elevation billboard plane. The latest pass separates the six overlays
into 18 crown sectors belonging to 17 supporting trees. Each sector has major
branches, forks, shoots and terminal twigs, 2 small internal foliage masses,
and many pointed leaf meshes: 15,930 sprays containing 143,370 leaves overall.
The foreground Arbre05 root remains inferred. The previous rounded-clump scene
is preserved as `sherwood-before-branch-canopies.blend`.

Branches attach to supporting trunks, with excess stump tips trimmed at forks.
Leaf sprays clear the central stems and use local opaque texture samples on
rear faces. Both front and rear retain animated source-canvas alpha to keep
original holes and borders. Two silhouette-fitting rounds add sparse samples
at measured gaps. At the end of the initial branch/leaf pass, first-frame alpha
coverage was 99.66–99.88% across the six
overlays, with zero extra opaque pixels; this measures silhouette registration,
not recovered 3D accuracy. Hidden branch structure and crown thickness remain
inferred from the single picture.

The subsequent depth correction removes two causes of the side-view slab shape:
depth no longer depends on the thinner screen-space silhouette dimension, and
leaves no longer share a reference-camera-biased orientation. Crowns are centered
on world-vertical tree axes; horizontal spread sets their depth. Camera-ray
placement retains the measured spray positions in the original view. Horizontal
principal-axis ratios improve from 0.37–0.93 to 0.79–0.96. These ratios measure
plan-view fullness, not fidelity. `fuller-crowns/` records before/after depth
metrics and the successful topology checks. The native pre-correction scene is
`sherwood-before-fuller-crowns.blend`.

`branch-canopies/` contains original-camera, east/west, overhead, foreground,
branch-only and wireframe renders, plus topology and silhouette reports.
The checked branch/leaf geometry and trimmed trunks have no non-manifold edges
or zero-area faces.

## Animated turntable

`turntable/sherwood-turntable.mp4` is a 12-second, 1440 × 1080, 25 fps full
rotation. Textured, solid and wireframe passes use matching camera and animation
frames, with three 20-frame swipe transitions. A final swipe returns to textured.
The separate **Sherwood Turntable** scene has camera keys on frames 1–301.
All six original tree atlases and all fourteen ambient overlays animate at their
authored delays. Gentle inferred branch sway is added to presentation copies
so foliage also moves in solid and wireframe views. Ambient sprite cards are
shown only in the textured pass; their rectangles are omitted from geometry
inspection passes. Camera bookmarks and original reconstruction objects remain
in their existing scenes.

Run `extract_turntable_fx.py` with normal Python, then `turntable.py` through
MCP once to create the scene. Load that module with `__name__='turntable'` for
subsequent calls to `render_frame(frame, mode)`, using `modes_for_frame(frame)`
for frames 1–300. Run `compose_turntable.py` with normal Python afterward; it
requires Pillow, NumPy and ffmpeg, composites the swipes, checks every frame's
alpha bounds, and saves an MP4, poster, contact sheet and validation JSON.

The newer `turntable-fast/sherwood-before-after.mp4` is a tighter 3:2 preview
at 960 × 640 and 15 fps, with edge cropping explicitly allowed. Its fixed center
split shows the untouched imported baseline on the left and the depth-corrected
refinement on the right. It samples the same 25 Hz authored animation timeline.
Both sides share the exact camera action; paired mode swipes run simultaneously
within the two halves. The baseline has its original 124 mesh objects and no
foliage added by the refinement passes.

For this version, run `turntable.py` through MCP with `FAST_PREVIEW=True` in
the execution scope. Then create the matched baseline scene with both
`FAST_PREVIEW=True` and `ORIGINAL_BASELINE=True`. Load the module with the same
flags and `__name__='turntable'` to render each side using `render_frames()` and
`modes_for_frame(frame)`. Fast rendering uses 8 EEVEE samples, Workbench FXAA,
and short native animation batches to avoid reinitializing the render engine
for every still. Existing pass files can be reused only when their scene and
settings are unchanged. Finish with `compose_turntable.py --fast --split`.

## Geometry passes

| Collection | Reconstruction |
| --- | --- |
| 03–06 | Long suspension bridge, ladder oak, three radial platforms, furniture supports |
| 08 | 22 tapered/fluted trunks, root buttresses and traced branches |
| 09 | Central oak fork; plank-built hut, peaked roof, porch rails and ladders |
| 10 | 18 branch-bearing crown sectors, 143,370 leaves and animated alpha atlases |
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
8. Preserve a native backup, remove only generated collection 10 and its objects,
   then run `refine_branch_canopies.py`. The checked-in `foliage_fit_samples.json`
   supplies measured residual samples. Keep collection 11's ambient references.
9. `render_canopy_masks.py`, then normal Python
   `fit_canopy_silhouettes.py --measure-only`; `inspect_branch_canopies.py` through MCP.

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
- Further shape concealed crown depth and branch taper with additional art
  direction; single-view silhouette agreement cannot verify rear geometry.
- Finish small bushes, baskets, utensils, chimney masonry and untouched
  background obstacle shapes.
- Further fit the ladder oak and camp outlines against pixel residuals.
- Improve filled ground and concealed material transitions; separate baked
  lighting where useful.
- Map multipart meshes deliberately to editor obstacle nodes before exporting
  a replacement GLB. The native Blender scene is the deliverable of these passes.
