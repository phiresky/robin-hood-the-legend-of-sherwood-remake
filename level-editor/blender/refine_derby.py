"""Integrate reviewed Derby detail recipes, refresh projection, and stage exports.

Requires the grouped Derby checkpoint with the staircase pass already applied.
Run in the primary Blender session after reviewing isolated worker outputs.
Publication into the editor library is a separate explicit operation.
"""
import json
import runpy
from pathlib import Path


def stage(repository_root, output_dir, layers_manifest, include_watchtower=False):
    import bpy
    root = Path(repository_root).resolve()
    output = Path(output_dir).resolve()
    if output.exists():
        raise FileExistsError("Use a fresh stage directory")
    scripts = root / "level-editor/blender"
    manifest = Path(layers_manifest).resolve()
    source = manifest.parent / json.loads(manifest.read_text())["sources"]["exterior"]
    if not source.is_file():
        raise FileNotFoundError(source)
    bpy.context.window.scene = bpy.data.scenes["Derby Refinement"]
    output.mkdir(parents=True)

    def load(name):
        return runpy.run_path(str(scripts / (name + ".py")))

    projection = load("reproject_map")
    projection["restore_projection"]("Derby")
    changes = {
        "keep": load("derby_architecture")["refine"](),
        "gate": load("derby_gatehouses")["refine"](source),
        "cottage": load("derby_cottages")["refine"](),
        "furniture": load("derby_furniture")["refine"](),
    }
    changes["table_projection_cells"] = load("subdivide_projection_faces")["subdivide_tables"]()
    if include_watchtower:
        changes["watchtower"] = load("derby_east_hall")["refine"]()
    changes["names"] = load("group_assets")["sync_asset_names"](root / "level-editor/shared/assets/derby.json")
    report = projection["reproject_layers"](manifest, output / "reprojection")
    exporter = load("export_editor")
    map_export = exporter["export_editor"]("Derby", output / "derby.scene.glb")
    asset_export = exporter["export_asset_library"](
        "Derby", output / "assets", root / "datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.json")
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    result = {"changes": changes, "map": map_export, "assets": asset_export,
              "projection": {key: report[key] for key in ("projected_faces", "fallback_faces", "limitations")}}
    (output / "stage.json").write_text(json.dumps(result, indent=2) + "\n")
    return result
