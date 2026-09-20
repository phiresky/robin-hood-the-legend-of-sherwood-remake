"""Stage a map and its asset library with optional inferred hidden textures.

Load the saved refinement blend, call stage(), inspect the output, then use the
pipeline publishing commands. This module never overwrites the loaded blend.
"""
import hashlib
import json
from pathlib import Path


def geometry_signature(map_name):
    import bpy
    digest = hashlib.sha256()
    for obj in sorted(bpy.data.collections[map_name + " Working"].all_objects, key=lambda obj: obj.name):
        if obj.type != "MESH":
            continue
        record = [obj.name, obj.get("source_node"), obj.get("asset_group"),
                  obj.parent.name if obj.parent else None,
                  list(sum((tuple(row) for row in obj.matrix_world), ())),
                  [tuple(v.co) for v in obj.data.vertices],
                  [tuple(p.vertices) for p in obj.data.polygons]]
        digest.update(json.dumps(record, separators=(",", ":")).encode())
    return digest.hexdigest()


def stage(map_name, manifest_path, output_dir, level_path, *, hidden_fill="synthesized",
          texels_per_unit=1, source_mask_manifest=None):
    import bpy
    import export_editor
    import reproject_map
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=False)
    before = geometry_signature(map_name)
    projection = reproject_map.reproject_layers(
        manifest_path, output / "reprojection", hidden_fill=hidden_fill,
        texels_per_unit=texels_per_unit, source_mask_manifest=source_mask_manifest)
    after = geometry_signature(map_name)
    if before != after:
        raise RuntimeError("Texture refresh changed geometry, naming, or grouping")
    bpy.ops.wm.save_as_mainfile(filepath=str(output / "worker.blend"))
    report = {"geometry_before": before, "geometry_after": after,
              "hidden_fill": hidden_fill,
              "known_texels": sum(item["known_texels"] for item in projection["ownership_bakes"]),
              "unknown_texels": sum(item["unknown_texels"] for item in projection["ownership_bakes"]),
              "missing_donor_objects": [item["object"] for layer in projection["ownership_bakes"]
                                        for item in layer["objects"] if item.get("missing_donor")],
              "map": export_editor.export_editor(map_name, output / (map_name.lower() + ".scene.glb")),
              "assets": export_editor.export_asset_library(map_name, output / "assets", level_path)}
    (output / "stage.json").write_text(json.dumps(report, indent=2)+"\n")
    return report
