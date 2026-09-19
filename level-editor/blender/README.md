# Projection-mapped map refinement

All Blender operations run through Blender MCP. These scripts accept explicit
paths and preserve the editor's source obstacle IDs. Generated `.blend`, GLB,
render and library files stay in the ignored `work/` and `library/` directories.

## Derby checkpoint

`work/derby-refinement/derby-refinement.blend` contains a hidden imported baseline,
the working map, 30 named asset parents and reference/oblique/detail cameras.
The 270 source obstacles have complete ownership in `shared/assets/derby.json`.
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

The editor's `library/scenes/derby-volumes.scene.glb` and
`derby.level3d.json` are published together. Reopen Derby to load a new revision.
Names/groups live in the document; mesh IDs remain stable for transforms and
second-click part selection. Untouched legacy generated documents upgrade on
load. Saved custom ownership/transforms are retained.

## Running scripts through MCP

Load a module without invoking it implicitly:

```python
path = ROOT / "level-editor/blender/export_editor.py"
scope = {"__file__": str(path), "__name__": "export_editor"}
exec(compile(path.read_text(), str(path), "exec"), scope)
result = scope["export_editor"]("Derby", STAGING / "derby.scene.glb")
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
  and visibility sets. See [reprojection.md](reprojection.md) for limitations.
- `export_editor.export_editor(map_name, output_path)` exports the current visible
  working meshes, including evaluated modifiers. Hidden baseline/reference
  geometry and cameras are excluded. Each named asset parents stable obstacle
  nodes; each obstacle can have multiple named geometry components.
- `export_editor.export_asset_library(map_name, output_dir, level_path)` exports
  **all** named assets as standalone GLBs, descriptors and an index. Use a fresh
  staging directory. The map export stays in map coordinates; standalone assets
  are centered horizontally with their lowest geometry at local ground height.

## Publishing reviewed exports

`refine_derby.stage(repository_root, fresh_output_dir, layers_manifest,
include_watchtower=True)` reruns the reviewed detail recipes, projection,
map export, and all standalone exports from an existing grouped checkpoint.
It saves the main blend after staging; publication remains explicit.

From the repository root:

```sh
# First publication of named ownership on an unedited reconstruction:
node level-editor/pipeline/src/publish-asset-catalog.ts \
  level-editor/library/scenes/derby-volumes.scene.json \
  datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.json

# Publish new geometry while retaining the document and updating its fingerprint:
node level-editor/pipeline/src/publish-refined-map.ts \
  level-editor/work/derby-refinement/pass7-publish/derby.scene.glb \
  level-editor/library/scenes/derby.level3d.json

# Merge standalone models into the reusable library:
node level-editor/pipeline/src/publish-model-assets.ts \
  level-editor/work/derby-refinement/pass7-publish/assets \
  level-editor/library/3d-assets
```

Map publication retains the prior GLB and document under `scenes/backups/`.
Library publication validates staged assets and merges the index with other maps.
It replaces files for the same asset IDs; keep the staging pack for each revision.

## Standalone model contract

`library/3d-assets/index.json` lists each asset's ID, display name, source map,
descriptor and model paths. `<id>/asset.json` is version 1,
`kind: "projection-mapped-asset"`, with local bounds, source placement origins,
stable component nodes, and complete obstacle records in local game coordinates.
All GLBs contain their textures. Scene-frame mesh children are Z-up; the `map`
wrapper rotates them to standard glTF Y-up. Restore `source_origin_scene` after
removing that wrapper to reassemble the source scene exactly.

Component metadata records projection layers and patch associations. Map exports
retain all patch records; standalone assets retain associated patches. Cover PNGs
are embedded in reveal metadata. Patch state coordinates remain in the source
game frame, explicitly separate from local collision records. Sight-state changes
do not imply removal of rendered walls. Automatic editor cutaway behavior remains
to be implemented.

## Independent Blender workers

Run separate jobs against copied checkpoints with `run_worker.py`:

```sh
/usr/bin/blender --background --factory-startup --python-exit-code 1 \
  --python level-editor/blender/run_worker.py -- \
  --source level-editor/work/derby-refinement/derby-refinement.blend \
  --job path/to/job.py --output-dir path/to/fresh-worker-directory
```

The worker saves its own blend, log, and result and verifies the source fingerprint.
Integrate reviewed scripts in the primary session before reprojection and export.

This is a 3D model catalog, separate from the older 2D cutout library schema.
It does not invent segmentation/fit scores or navigation geometry. The current
editor loads complete maps and selects their groups; a new-map placement palette
must consume this catalog and persist its added model references. That palette
is not implemented by this refinement/export pass.

TODO: crop/repack per-asset atlases to reduce duplicate embedded texture data;
finish asset previews and new-map placement UI; expand geometry work beyond stairs.
