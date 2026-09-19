# Refreshing projection after geometry edits

Run `reproject_map.py` through Blender MCP after the last geometry edit, before
`export_editor` and `export_asset_library`. The function changes working meshes
and materials in memory; saving the checkpoint and publishing exports remain
explicit steps.

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
an audited interior-only BVH: this removes exterior shells only from the source
visibility calculation, never physically deletes them. Derby's covers include
front-wall cutaways, so dropping roofs alone does not reveal every interior.
Stable IDs are `source_node` values such as `building-212`. Do not equate
collision obstacles or image overlap with verified interior ownership. Retain
fallback materials on unaudited interior surfaces.

Bake active mesh modifiers first. Meshes must already have valid fallback atlas
materials and UVs. Each map must use a `<Map> Working` collection with Z-up world
coordinates and map-pixel units. The source image dimensions determine framing;
nonstandard camera framing is not supported by this function.

The refresh computes UVs from current world-space geometry and rebuilds a BVH of
the selected working occluder meshes. Source-facing polygons receive the packed source
image only when all visibility samples are unoccluded and their vertices lie
inside the source image. Larger triangles receive more samples, up to the
configurable subdivision cap. Back-facing, partly hidden and out-of-image faces
retain their previous materials and UVs. This avoids indiscriminately painting
foreground buildings onto hidden walls. A face material attribute retains the
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
runs one exterior pass and one interior-only pass per patch. It rejects maps
without an authored receiver review. Pixel-overlap candidates are never promoted
to interiors; ambiguous candidates receive covered artwork and keep explicit
ambiguous metadata. No objects are hidden or removed.

`layers-report.json` includes every pass, receiver lists, unresolved candidates,
source/geometry fingerprints and manifest/recipe hashes. Working objects retain
their assigned projection layer, source path, hashes and projected/fallback face
counts. Original sight activation metadata remains independent of rendering.
Matching source images are reused across interior materials and reruns to avoid
packing redundant copies in exports. Ground stays on its existing atlas.

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
