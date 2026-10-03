# Map and asset library format

A published map is `library/scenes/<map>.rhlos-map.json` plus references to the
same local assets offered by the editor palette. There is no separate map model,
map-coordinate asset scene, or automatic whole-map GLB fallback.

## Files

```
library/
  scenes/<map>.rhlos-map.json
  scenes/backups/...
  3d-assets/index.json
  3d-assets/<source-map>/<asset-id>/asset.json
  3d-assets/<source-map>/<asset-id>/model.glb
  3d-assets/blobs/<sha256>.bin
  3d-assets/blobs/<sha256>.png
  3d-assets/blobs/<sha256>.jpg
```

`3d-assets/index.json` is a generated cache. The source of truth is the recursive
set of `asset.json` files and their neighboring models, not the previous index.
Descriptors supply IDs, names, source maps, scene names, tags, asset types, and
editor usage. The index includes each descriptor's SHA-256 and a trimmed editor
view: part names, local collision footprints, appearances, model scenes, and
external resource pins. The editor reads this catalog for map loading, insertion,
and previews without opening individual `asset.json` files. Reconstruction and
publication tools continue to use the full descriptors and their review evidence.
The generator sorts entries by ID and discovers `lossy.glb` and
`preview.glb` (or `<model-stem>.lossy.glb` / `<model-stem>.preview.glb` for named
models). Duplicate IDs, incomplete descriptors, and missing models are errors.
Hidden, backup, shared-blob, and symlinked directories are excluded.

Add or remove an asset directory, then run `pnpm library:index` from `level-editor/`
to regenerate the catalog. Publication scripts regenerate it automatically after
installing payloads. Editing the generated index does not change the assets;
regeneration also works when the previous index is missing or corrupt.
The index identifies assets and their descriptors, including their relative paths.
Source-map directories use lowercase names (for example `derby/`); asset IDs remain
stable. Consumers resolve the index paths instead of constructing paths from IDs.
A descriptor records local
collision footprints, stable part names, the model, and its pinned resources.
`model_scene` selects a named reusable appearance in a multi-scene model.
`state_variants` and `standalone_variants` retain their existing initial/applied
semantics. Different appearances share geometry and image payloads where their
bytes match.

Each asset uses one GLB, including its named appearances. Private payloads and
small shared payloads are embedded. A payload stays external only when sharing
between distinct assets saves at least 256 KiB: `bytes × (asset count − 1)`.
Sharing between appearances of a single asset happens inside its GLB and never
requires an external file. Models with `resources: []` are self-contained.

`blobs/` holds only the worthwhile shared buffers and textures, such as the large
reconstruction atlases. GLBs that use these files reference ordinary relative
`../../blobs/...` URIs; descriptors and saved references pin library-relative paths
and SHA-256 hashes. Packaging preserves texture encoding and accessor bytes.
Optional preview models are derived browser thumbnails, never map geometry.
`refinement/blender/lossy_assets.py` builds `preview.glb` from the lossy model (or the
model when there is none): simplified, meshopt-compressed geometry and an AVIF texture of
about 1 texel per 8 map pixels. `preview.glb.receipt.json` binds its `source_model` bytes.
An optional `lossy_model` (for example `<source-map>/<asset-id>/lossy.glb`) is a
derived lossy display copy of `model`: same nodes, extras, scenes and materials, one
re-baked texture atlas (EXT_texture_avif), quantized vertices (KHR_mesh_quantization)
and no normals on unlit primitives (nothing is lit yet).
Geometry is then losslessly encoded with `KHR_meshopt_compression` (vertex codec v1)
when the complete GLB becomes smaller. This step preserves accessor values, index
order, textures and scene metadata; it does not simplify or requantize geometry.
Previews also use KHR/v1, after their existing simplification and texture processing.
The Python GLB reader decodes meshopt before inspecting attributes.

Existing derivatives can be migrated without rebaking with
`python3 refinement/recompress_library.py --work work/meshopt-migration --apply`
(from `level-editor/`). Omit `--apply` to stage only. The work directory must be
fresh; installation checks source hashes under the publication lock and retains
backups and a restore manifest. Preview receipts retain their generation fingerprint
so migrating compression does not certify outdated preview-generation settings.

`<lossy_model>.receipt.json` records the SHA-256 of the `model` bytes it was built
from (`source`) and of itself (`output`). Every asset-index writer uses
`refinement/asset_index.py::write_asset_index`: it generates entries from directories,
checks both hashes for every discovered lossy model, then atomically replaces the
index. Missing models,
missing or malformed receipts, stale sources, and changed output bytes fail publication
without replacing the existing index. Source-only entries need no receipt.

Publication transactions validate proposed staged files together with unchanged live
assets before installation, and validate again when writing the live index. Callers
retain their existing locks and rollback backups. The generator requires Python 3.11+.
Historical transaction rollback restores the original files and index from backups.

Saved maps keep pinning `model`; the editor loads the indexed lossy display copy
directly, without fetching receipts. If no `lossy_model` is declared, it loads and
checks the original model. Receipts remain build/publication metadata. To validate
an existing library without writing anything, run from `level-editor/`:

```sh
pnpm library:index --check
```

## Local assets and placed instances

Mesh children use a local Z-up frame; the glTF `map` wrapper converts to Y-up.
Each asset descriptor records its export pivot as `source_origin_scene` so a
revised asset can be rebased without adding export provenance to the map. Asset
files contain no hinge positions in map coordinates or mission-state records.
Source obstacle/profile identifiers may remain as provenance. A reusable
appearance switch has an asset-local ID.

The map's `assetSources` pins each catalog asset. When an asset has multiple
appearances in the scene, one source entry holds the descriptor pin and an
`appearances` array of model pins. Saved source records omit `model_scene` and
`resources`: the verified descriptor supplies the scene selector and external
resource pins when the map is loaded. A version 2 map stores one entry
in `placements` for each placed asset group. Its `assets` array names each
catalog asset once; `appearances` records which of its forms are present in
that placement. The placement retains its ID, name, and transform. The pinned
descriptors supply the default parts, local transforms, collision footprints,
names, and visibility. The editor expands placements into its normal group
and object model when loading.

A descriptor part is an obstacle part (`source_obstacle` plus
`obstacle_local_game`), a mission part (`mission_profile` plus editor-only
`obstacle_local_game`), or authored scenery. Authored scenery is a `foliage-*` or
`scenery-*` node with `scenery: true` and no obstacle, footprint or mission
profile, for example `{"node": "foliage-oak", "name": "Painted tree",
"scenery": true}`. Its GLB part node carries `scenery: true` in its extras. A
placed scenery part has `kind: "scenery"`, `source: {map}` and no `obstacle`. It
rotates about its asset-local origin, and its group pivot ignores it. The index
entry is an ordinary asset entry. Game baking rejects scenery until the compiler
can place its geometry.

Only changed parts appear in a placement's `parts` object, keyed by descriptor
part node. Each entry stores fields that differ from the descriptor defaults,
including an edited transform or collision shape. `removed`
lists deleted descriptor parts; `copies` describes extra instances of a part.
Mixed-asset placements use full `asset:<asset-id>:<part-node>` keys. `idMode`
selects the existing object ID convention, and an exceptional object ID appears
as a part override. The placement array defines scene order; parts within a
placement follow descriptor order, with copies last. Separate `--state-*` asset
source records are no longer accepted in version 2 maps. Old version 1 maps
remain readable and expand into the same editor model.

Placement `patches` maps asset-local appearance IDs to mission patch IDs. For
example, `{ "derby-keep-main-hall": { "appearance-1": "patch-000" } }` binds
all nodes controlled by that asset appearance without repeating their names.
For static initial/applied asset scenes, the local `state` slot controls parts
unique to each endpoint; shared parts remain visible. The editor applies the
mapping to placed clones. Asset component roles and publication evidence stay
with their source data. The map keeps
only reveal patch IDs and names in `sceneMetadata.reveal.patches` for editor labels;
game patch states and review frames stay in their source manifests. The map
does not need export origins for editing or rendering; publication reads those
from the old and new pinned asset descriptors when an asset revision changes
its local origin. Terrain is a pinned background catalog asset in `sceneAssets`.

Coordinates use ordinary floating-point values. The migration verifies collision
footprints and flags and compares rendered views, allowing small rasterization
differences from local-coordinate roundoff. No residual-coordinate extension or
special precision data is stored in assets.

## Authoring and compilation

**New map** creates an unbounded canvas (`size: null`); choosing dimensions is not
a prerequisite. `exportBounds: [x, y, width, height]` is an optional output crop,
not an editing boundary. Content may intentionally extend beyond it. Without an
explicit crop, an authored-map compiler should derive output bounds from content.
The current reconstruction baker cannot compile placed catalog geometry or custom
crops; saving/publishing the editor document is separate from game-file baking.

## Export, verification, and cleanup

The refinement map exporter writes this format directly. Selected reviewed assets
and map instances share one canonical model; unselected map groups are also
exported as reusable local catalog assets. Intermediate standalone GLBs and their
placement evidence are retained only in the staging backup/evidence area.

Publication verifies the complete referenced resource graph, installs asset files
before the map/index, and uses hash guards, a library lock, backups, and rollback.
A changed pin is an error; the loader does not silently substitute a newer asset.
See [the refinement procedure](../refinement/PROCEDURE.md#map-publication-format).

Retired whole-map scenes, redundant embedded models, and unreferenced payloads
belong under `library/scenes/backups/`. Cleanup must retain every file referenced
by any active map, catalog appearance, or preview. Ongoing Blender workers and
review evidence are independent of this library cleanup.

For an existing split-scene library, stage conversion with
`python3 refinement/unify_map_assets.py library work/canonical/staged`, then run
`node pipeline/src/place-canonical-assets.ts work/canonical/plan.json` from
`level-editor/`. Review the placement proof and rendered comparisons before
running `python3 refinement/apply_canonical_library.py work/canonical/plan.json
--apply`. Without `--apply`, this command validates source guards and the complete
active graph and reports the proposed cleanup. It archives superseded files and
restores previous files if installation fails.

To repack an existing local catalog using the hybrid policy, run
`python3 refinement/hybrid_library.py library work/hybrid/staged`, review its
report and rendered comparisons, then run
`python3 refinement/apply_canonical_library.py work/hybrid/plan.json --apply`.
The packer also accepts an existing hybrid catalog and rediscovers sharing from
embedded payloads, placing every asset under its source-map directory and updating
map pins. `--min-savings` sets the byte threshold. The refinement map
exporter runs this packing step automatically across its staged catalog;
library-wide repacking can additionally find sharing across separate publications.

The editor reads published files over HTTP, using `3d-assets/index.json` for the
palette. Its static serving preparation generates `scenes/index.json` (an array
of map JSON filenames) and excludes backups. Browser saves are OPFS copies, never
writes to this library; **Download** exports the current map JSON for external use.
