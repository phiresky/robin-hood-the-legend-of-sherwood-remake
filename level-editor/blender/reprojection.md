# Refreshing projection after geometry edits

Run `reproject_map.py` through Blender MCP after the last geometry edit, before
`export_editor` and `export_asset_library`. The function changes working meshes
and materials in memory; saving the checkpoint and publishing exports remain
explicit steps.

Use `reproject_layers` for final reviewed output. It now finishes each layer with
`source_projection_bake.bake`: every atlas texel checks the first surface hit at
its continuous source-camera coordinate. Hidden texels become neutral shaded
gray; they never retain an older projected atlas. Explicitly preserved authored
or generated materials and the cleaned ground are retained. The low-level
`reproject_map` call below is a preliminary face-assignment pass, not sufficient
on its own to guarantee source-pixel ownership.

```python
path = ROOT / "level-editor/blender/reproject_map.py"
scope = {"__file__": str(path), "__name__": "reproject_map"}
exec(compile(path.read_text(), str(path), "exec"), scope)
result = scope["reproject_map"](
    "Derby",
    STAGING / "interior-layers/covered.png",
    STAGING / "reprojection.json",
    elevation_deg=35.0,
    receiver_nodes=EXTERIOR_NODE_IDS,
    occluder_nodes=EXTERIOR_NODE_IDS,
    projection_label="exterior",
)
```

**Choose the correct artwork layer first.** Derby's raw Day image contains
revealed interiors; the exterior must use the covered composite produced by
`pipeline/src/export-interior-layers.ts`. Its manifest retains exact cover
sprites, bounds and explicit sight-obstacle swaps. Use separate calls with
disjoint `receiver_nodes`, appropriate `occluder_nodes` and distinct
`projection_label` values for exterior and each audited interior layer. Interior
passes use `revealed.png` and omit cover geometry from their occluder lists.
Before changing an existing layer partition, call
`restore_projection("Derby")` once to restore all saved atlas face assignments,
then run the disjoint exterior and interior passes. Interior visibility can use
an audited state-specific BVH: retained exterior battlements and walls must still
occlude interior receivers. This changes source visibility only, never physically
deletes geometry. Derby's covers include
front-wall cutaways, so dropping roofs alone does not reveal every interior.
Stable IDs are `source_node` values such as `building-212`. Do not equate
collision obstacles or image overlap with verified interior ownership. Retain
explicit uncertainty on unaudited interior surfaces.

Bake active mesh modifiers first. Meshes must already have valid fallback atlas
materials and UVs. Each map must use a `<Map> Working` collection with Z-up world
coordinates and map-pixel units. The source image dimensions determine framing;
nonstandard camera framing is not supported by this function.

The refresh computes UVs from current world-space geometry and rebuilds a BVH of
the selected working occluder meshes. Source-facing polygons receive the packed source
image only when all visibility samples are unoccluded and their vertices lie
inside the source image. Larger triangles receive more samples, up to the
configurable subdivision cap. Back-facing, partly hidden and out-of-image faces
retain their previous materials and UVs in this preliminary pass. Those old
materials can themselves contain duplicated projection, which is why the final
ownership bake is required. A face material attribute retains the
fallback assignment so the function can run again after subsequent edits.

The report records source-image and geometry hashes, projected and fallback face
counts, fallback reasons and visibility-ray counts per object. Exported materials
explicitly reference the new UV layer; existing atlas materials keep their
original active-render UV layer. Both map and standalone-asset GLBs therefore
carry refreshed projection coordinates and packed textures.

Visibility is sampled, not a guarantee for every source pixel. Narrow occluders
between samples can be missed; review reference and oblique textured renders.
The source painting also cannot reveal genuinely concealed surfaces. Ground is
left on its existing cleaned atlas. When refinements expose ground previously
covered by structures, use a separate ownership-mask and texture-synthesis pass;
this script does not claim to regenerate that ground. For topology edits on an
already refreshed mesh, preserve the face fallback-material attribute, or assign
valid fallback materials before clearing that attribute and rerunning.

## Audited layer orchestration

For maps with an authored recipe in `interior_layers.py`, prefer:

```python
result = scope["reproject_layers"](
    STAGING / "interior-layers/layers.json",
    STAGING / "reprojection",
)
```

This preflights distinct covered/revealed image paths and matching dimensions,
valid patch IDs, disjoint receiver ownership, present/visible source IDs and
fallback materials. It restores prior passes, annotates audited roles, then
runs an exterior pass and a pass per interior patch, each with independently
reviewed receiver and occluder sets from `projection_occluders`. It rejects maps
without an authored receiver review. Pixel-overlap candidates are never promoted
to interiors; ambiguous candidates receive covered artwork and keep explicit
ambiguous metadata. No objects are hidden or removed.

`layers-report.json` includes every pass, receiver lists, unresolved candidates,
source/geometry fingerprints and manifest/recipe hashes. Working objects retain
their assigned projection layer, source path, hashes and projected/fallback face
counts. Original sight activation metadata remains independent of rendering.
Matching source images are reused across interior materials and reruns to avoid
packing redundant copies in exports. Ground stays on its existing atlas.

`ownership_bakes` records the final known/unknown texels, atlas dimensions and
visibility checks; preliminary projected/fallback face counts are not proof of
final texture ownership. Optional `ownership_nodes` scopes a worker's expensive
bake to its assigned parts; final publication defaults to all non-ground
receivers. Full-map bakes can take several minutes and should use a background
Blender process when they exceed the MCP command timeout. Geometry remains
unchanged. Existing source-only bake materials are refreshed on rerun; other
explicit `projection_preserve` materials retain their authored content.

For the normal editor export, `hidden_fill="synthesized"` fills unknown texels
from observed donor patches belonging to the same logical asset and reveal
layer. Similar surface inclinations are preferred, then the same mesh. The
installed `~/.cargo/bin/texture-synthesis` generator expands fully observed
patches into cached 128×128 tiling textures with deterministic single-threaded
seeds; jobs run with bounded parallelism. Donors smaller than 16 pixels use an
explicitly reported mirrored fallback because the generator cannot handle those
inputs reliably. Missing donors remain neutral and are listed in the report.
Generated results are an appearance approximation, never source evidence.

The default `hidden_fill="neutral"` remains unchanged for refinement workers.
In synthesized exports, RGBA alpha records ownership (one observed, zero
inferred), while the material remains opaque and uses `KHR_materials_unlit`.
`source_ownership_fill` material extras allow the editor to hide inferred RGB
without rebuilding the map. Linear texture interpolation is used. Accepted
authored/AI materials are still protected by `projection_preserve`.

`refresh_editor_textures.stage(map_name, manifest_path, fresh_output_dir,
level_path)` stages a saved blend, the full scene, all named assets, and reports.
It asserts that geometry, transforms, stable names and grouping are unchanged.
`synthesize_owned_atlases.synthesize(map_name, output_dir)` can regenerate
inferred RGB in existing ownership atlases without rerunning visibility rays;
it asserts exact preservation of every observed texel.

Both layer orchestration and `source_projection_bake.bake` accept an optional
`source_mask_manifest`. Reviewed assignments in that manifest may reference
converter-generated `*.rhp.d/masks/manifest.json` PNG inventories. Unassigned
objects remain unconstrained; assignments are never inferred from overlapping
bounding boxes. Each active projection label requires the exact source image
hash, an explicit state description, and `reviewed: true` on every assignment.
Source-node assignments override asset-group assignments. These masks constrain
texture evidence, not geometry; see `occlusion_constraints.py` for the schema.

Derby's upper gatehouse interior includes the surrounding upper masonry and
battlements in its occluders. Excluding them previously assigned the same painted
crenellations to both the battlements and the roof behind them. Other room covers
contain partially removed facades and retain explicit audit limitations; a whole
object is not automatically removed just because it intersects a cover sprite.

`reprojection_selftest.py` exercises front-facing projection, occlusion,
back-facing fallback, original UV preservation and repeat-run geometry/slot
updates in a separate background Blender process. It also exercises separate
covered/revealed artwork on overlapping surfaces and verifies the exported
glTF material's selected UV accessor against the refined geometry's projection.

Keep room floors in interior visibility tests. Furniture collision columns can
extend below a room floor; those buried faces must remain occluded. Refine the
render mesh to the audited room floor before refreshing projection, while
retaining its original collision record for export.

Derby's two banquet tables also use
`subdivide_projection_faces.py::subdivide_tables()` after furniture floor
clipping and before `reproject_layers`. Bounded subdivision lets the visible
wood and tablecloth project around benches and stools, while locally hidden
cells retain the fallback atlas. It preserves the closed shell and source UVs;
repeat calls with the same spacing do not add more faces.
