# Per-map Blender recipes

This directory holds map-specific recipes only: `blender/<map>/` for Leicester,
Lincoln, Nottingham and Sherwood, and the Derby recipes at this top level. All
shared tooling (including `setup_map`, `render_views`, `reproject_map`,
`run_worker` and the export/review/bake helpers) lives in
`../refinement/blender/`; see [the refinement index](../refinement/README.md).
Generated `.blend`, GLB, render and library files stay in the ignored `work/`
and `library/` directories.

The rest of this file is the Derby history. For its progress and known defects,
see [derby-review.md](derby-review.md).
Use the [refinement worker workflow](../refinement/refinement-workflow.md) for new passes:
review scene grouping first, then prepare one isolated workspace per asset with
source context and matching eight-view `input/` and `modified/` review packets.
The textured packets show original projected pixels and shaded unknown surfaces;
generated textures are excluded from the evidence used to refine geometry.

## Derby checkpoint

`work/needs-review/derby-refinement/derby-refinement.blend` contains a hidden imported baseline,
the working map, 30 named asset parents and reference/oblique/detail cameras.
The 270 source obstacles have complete ownership in `refinement/catalogs/derby.json`.
The Great Keep, East Hall, freestanding watchtower, cottages and curtain walls
select independently; roofs and supporting walls belong to the same asset.

Eleven staircase flights now have 123 physical treads and risers. Their UVs use
the existing atlas's reference-camera projection. Every added stair is closed,
with no non-manifold edges or zero-area faces. This is an initial detail pass:
crenellations, arches, rock relief, concealed surfaces and fine alignment of
individual painted risers still need work. Inferred step counts are editable in
`derby_stairs.py` and `derby_stair_details.py`.

Additional audited geometry includes seven keep/watchtower parapet notches,
the south gate arch, and a recessed west-cottage doorway. Each script retains
its hidden source mesh.
Use `sync_asset_names` to refresh reviewed furniture labels without reparenting.

The editor loads `library/scenes/derby.rhlos-map.json` and the canonical local assets
in `library/3d-assets/`. Reopen Derby after publication. Map instances use the same
models as palette insertion; the map JSON retains placement and mission bindings.

## Running scripts through MCP

Load a module without invoking it implicitly:

```python
path = ROOT / "level-editor/refinement/blender/export_editor.py"
scope = {"__file__": str(path), "__name__": "export_editor"}
exec(compile(path.read_text(), str(path), "exec"), scope)
result = scope["export_editor"]("Derby", STAGING / "derby.rhlos-map.json",
    level=level, map_settings={"size": None})
```

- `setup_map.setup_map(metadata_path, output_path)` imports a volume export into
  a new scene with independent baseline/working meshes. It uses the metadata's
  map size and camera elevation. Reference framing matches map pixels; other
  views fit the complete geometry.
- `render_views.render_views(scene_name, {label: camera_name}, output_dir,
  modes=("textured", "solid"), width=1200)` captures repeatable views and records
  camera matrices. Render settings, active scene and marker bindings are restored.
  Existing files are rejected so before/after evidence is not silently replaced.
- `group_assets.group_assets(catalog_path)` applies complete authored ownership,
  names and source IDs to a working scene. It verifies reparenting leaves world
  transforms unchanged. Run once after setup; author another catalog for each map.
- `derby_stairs.refine_stairs()` creates the first courtyard flight. Then apply
  `group_assets` and `derby_stair_details.refine()` for the remaining ten flights.
- `derby_architecture.refine()`, `derby_gatehouses.refine(source_image_path)`,
  and `derby_cottages.refine()` apply the audited architectural details.
- `derby_east_hall.refine()` cuts three watchtower embrasures. Their cut region
  has no open edges; pre-existing lower shell seams remain.
- `derby_furniture.refine()` trims fourteen furniture render columns at their
  audited room floors so buried sections cannot invalidate visible-side
  projection. Original collision descriptions remain separate and unchanged.
- `subdivide_projection_faces.subdivide_tables()` splits the two banquet table
  surfaces into smaller projection cells so nearby benches cannot force a whole
  visible side to keep its fallback material. Run after furniture clipping.
- `reproject_map.reproject_layers(manifest_path, report_dir)` refreshes projection
  after geometry changes. Generate its inputs with `export-interior-layers.ts`.
  Covered exterior artwork and revealed interior artwork have separate receiver
  and visibility sets. See [reprojection.md](../refinement/reprojection.md) for limitations.
- `export_editor.export_editor(map_name, output_path)` exports the current visible
  working meshes, including evaluated modifiers. Hidden baseline/reference
  geometry and cameras are excluded. Each named asset parents stable obstacle
  nodes; each obstacle can have multiple named geometry components.
- `export_editor.export_asset_library(map_name, output_dir, level_path)` exports
  **all** named assets as standalone GLBs, descriptors and an index. Use a fresh
  staging directory. These are intermediate local exports. Final map staging
  converts them to the shared catalog, with placement evidence outside assets.

## Publishing reviewed exports

`refine_derby.stage(repository_root, fresh_output_dir, layers_manifest,
include_watchtower=True)` reruns the reviewed detail recipes, projection,
map export, and all standalone exports from an existing grouped checkpoint.
It saves the main blend after staging; publication remains explicit.

Use `refinement/blender/stage_reviewed_publication.py` for reviewed publication,
then the handoff, asset and browser verification steps in the
[shared procedure](../refinement/PROCEDURE.md#map-publication-format). Prepare and
apply publication through `refinement/promote_staged_publication.py`. It installs
one canonical map/palette catalog, then the JSON manifest and merged index, with
hash guards, backups and rollback. Do not publish a whole-map GLB or independently
copy a second set of palette models.

## Standalone model contract

The [library format](../docs/library-format.md) describes descriptors, named local
appearances, shared payloads and per-instance mission bindings. Asset descriptors
contain local collision records and source IDs, but no source-map coordinates.
Each catalog asset is a GLB. Private payloads are embedded; `blobs/` retains only
payloads whose cross-asset sharing saves at least 256 KiB. Optional previews are
derived data.

## Independent Blender workers

Run separate jobs against copied checkpoints with `run_worker.py`:

```sh
/usr/bin/blender --background --factory-startup --python-exit-code 1 \
  --python level-editor/refinement/blender/run_worker.py -- \
  --source level-editor/work/needs-review/derby-refinement/derby-refinement.blend \
  --job path/to/job.py --output-dir path/to/fresh-worker-directory
```

The worker saves its own blend, log, and result and verifies the source fingerprint.
Integrate reviewed scripts in the primary session before reprojection and export.

This is a 3D model catalog, separate from the older 2D cutout library schema.
It does not invent segmentation/fit scores or navigation geometry. The editor's
asset palette supports manual placement and **New map** starts an unbounded canvas.
