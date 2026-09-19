"""Export refined geometry using stable editor part IDs and named asset parents."""
import base64
import json
import math
import struct
from pathlib import Path

import bpy
from mathutils import Matrix, Vector


def projection_metadata(source):
    """Carry surface ownership and projection provenance through glTF export."""
    values = {}
    for key in source.keys():
        if key.startswith(("reprojection_", "reveal_", "sight_patch_")) or key in (
                "projection_layer", "step_count", "crenellation_notches", "arch_segments",
                "gate_refinement", "cottage_refinement", "architecture_refinement", "embrasure_count", "refinement_recipe",
                "derby_furniture_floor_clip", "support_floor_source_node", "support_floor_scene_z",
                "projection_subdivision_spacing"):
            value = source[key]
            if hasattr(value, "to_list"):
                value = value.to_list()
            if hasattr(value, "to_dict"):
                value = value.to_dict()
            values[key] = value
    return values


def reveal_metadata(working, sources, include_all=False):
    path = working.get("reveal_manifest_path")
    if not path:
        return None
    manifest = json.loads(Path(path).read_text())
    nodes = {source["source_node"] for source in sources}
    patches = []
    for patch in manifest["patches"]:
        associations = {c["source_node"] for c in patch["coverage_candidates"]}
        associations.update(patch["sight_before"] + patch["sight_after"])
        if not include_all and not nodes.intersection(associations):
            continue
        graphic = patch["graphic"]
        portable_graphic = None
        if graphic:
            image_bytes = (Path(path).parent / graphic["image"]).read_bytes()
            portable_graphic = {"bbox_source_pixels": graphic["bbox"],
                                "image_data_uri": "data:image/png;base64," + base64.b64encode(image_bytes).decode("ascii")}
        patches.append({"id": patch["id"], "name": patch["name"],
                        "state_source_game": patch["state"],
                        "sight_before": patch["sight_before"], "sight_after": patch["sight_after"],
                        "graphic": portable_graphic,
                        "associated_source_nodes": sorted(nodes.intersection(associations))})
    return {"version": 1, "source_map": manifest["map"], "patches": patches,
            "scope": "complete map patch records" if include_all else "associated patches only; unrelated and unassigned source patches omitted",
            "coordinates": "Patch state remains in source game coordinates; standalone source_origin_game records the placement offset.",
            "visibility": "Sight obstacle state does not imply removal of rendered geometry; overlap is candidate association only."}


def export_editor(map_name, output_path, asset_id=None):
    working = bpy.data.collections[map_name + " Working"]
    output = Path(output_path)
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise FileExistsError(output)
    previous_scene = bpy.context.window.scene
    bpy.context.view_layer.update()
    depsgraph = bpy.context.evaluated_depsgraph_get()
    sources = [o for o in working.objects if o.type == "MESH" and not o.hide_render]
    if asset_id:
        sources = [o for o in sources if o.get("asset_group") == asset_id]
    if not sources or any(not o.get("source_node") for o in sources):
        raise ValueError("Every exported mesh must retain its editor source node")
    reveal = reveal_metadata(working, sources, include_all=asset_id is None)
    bounds = [o.matrix_world @ Vector(corner) for o in sources for corner in o.evaluated_get(depsgraph).bound_box]
    lo = Vector(tuple(min(p[i] for p in bounds) for i in range(3)))
    hi = Vector(tuple(max(p[i] for p in bounds) for i in range(3)))
    pivot = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z)) if asset_id else Vector()
    scene = bpy.data.scenes.new(map_name + " Editor Export")
    objects, meshes = [], []

    def node(name, parent=None, mesh=None):
        obj = bpy.data.objects.new("Export / " + name, mesh)
        scene.collection.objects.link(obj)
        obj.parent = parent
        obj["editor_node_name"] = name
        objects.append(obj)
        return obj

    try:
        root = node("map")
        # The editor reads Z-up part meshes below a glTF Y-up map wrapper.
        root.rotation_euler.x = -math.pi / 2
        groups, parts = {}, {}
        for source in sources:
            key = source["source_node"]
            mesh = bpy.data.meshes.new_from_object(source.evaluated_get(depsgraph),
                preserve_all_data_layers=True, depsgraph=depsgraph)
            mesh.transform(Matrix.Translation(-pivot) @ source.matrix_world)
            mesh.update()
            meshes.append(mesh)
            if key == "ground":
                node("ground", root, mesh)
                continue
            group_id = source["asset_group"]
            if group_id not in groups:
                group = node(source["asset_name"], root)
                group["asset_group"] = group_id
                groups[group_id] = group
            if key not in parts:
                part = node(key, groups[group_id])
                part["source_obstacle"] = int(key.split("-")[1])
                part["part_name"] = source["part_name"]
                parts[key] = part
            elif parts[key].parent != groups[group_id]:
                raise ValueError(f"Split asset ownership for {key}")
            piece = node(source.name, parts[key], mesh)
            piece["source_node"] = key
            for metadata_key, value in projection_metadata(source).items():
                piece[metadata_key] = value
        bpy.context.window.scene = scene
        bpy.context.view_layer.update()
        bpy.ops.export_scene.gltf(filepath=str(output), export_format="GLB",
            use_active_scene=True, export_yup=False, export_extras=True,
            export_animations=False, export_cameras=False, export_lights=False,
            export_image_format="AUTO")
        # Blender names are globally unique, even across scenes. Strip only our
        # export aliases in the JSON chunk; binary accessor offsets stay intact.
        data = output.read_bytes()
        length, kind = struct.unpack_from("<II", data, 12)
        if kind != 0x4E4F534A:
            raise ValueError("Expected a GLB JSON chunk")
        doc = json.loads(data[20:20 + length])
        for item in doc["nodes"]:
            extras = item.get("extras", {})
            if "editor_node_name" in extras:
                item["name"] = extras.pop("editor_node_name")
            if item.get("name") == "map" and reveal:
                item.setdefault("extras", {})["reveal"] = reveal
        chunk = json.dumps(doc, separators=(",", ":")).encode()
        chunk += b" " * (-len(chunk) % 4)
        binary = data[20 + length:]
        output.write_bytes(struct.pack("<4sII", b"glTF", 2, 20 + len(chunk) + len(binary)) + struct.pack("<II", len(chunk), kind) + chunk + binary)
        report = {"file": str(output), "assets": len(groups), "parts": len(parts),
                  "meshes": len(meshes), "steps": sum(o.get("step_count", 0) for o in sources)}
        if asset_id:
            descriptor = {"version": 1, "kind": "projection-mapped-asset", "id": asset_id,
                "name": sources[0]["asset_name"], "source_map": map_name, "model": output.name,
                "coordinates": "Z-up mesh children; Y-up glTF map wrapper; units are map pixels",
                "anchor": "horizontal bounds center at lowest geometry point",
                "source_origin_scene": list(pivot),
                "bounds_local_scene": {"min": list(lo - pivot), "max": list(hi - pivot)},
                "components": [{"name": source.name, "source_node": source["source_node"],
                                **projection_metadata(source)} for source in sources],
                "parts": [{"node": key, "name": obj["part_name"], "source_obstacle": obj["source_obstacle"]} for key, obj in parts.items()]}
            if reveal:
                descriptor["reveal"] = reveal
            output.with_name("asset.json").write_text(json.dumps(descriptor, indent=2) + "\n")
            report["asset"] = descriptor
        return report
    finally:
        bpy.context.window.scene = previous_scene
        for obj in objects:
            bpy.data.objects.remove(obj, do_unlink=True)
        for mesh in meshes:
            bpy.data.meshes.remove(mesh)
        bpy.data.scenes.remove(scene)


def export_asset_library(map_name, output_dir, level_path):
    """Export every named asset, local collision volumes, and merge the library index.

    Run into a fresh staging directory for each revision, then publish reviewed
    asset directories. Other maps in an existing index remain intact.
    """
    output_dir = Path(output_dir)
    level = json.loads(Path(level_path).read_text())
    working = bpy.data.collections[map_name + " Working"]
    ids = sorted({o["asset_group"] for o in working.objects if o.type == "MESH" and not o.hide_render and o.get("asset_group")})
    if not ids:
        raise ValueError("No named assets to export")
    if any((output_dir / key / "model.glb").exists() for key in ids):
        raise FileExistsError("Asset output exists; use a fresh staging directory")
    index_path = output_dir / "index.json"
    index = json.loads(index_path.read_text()) if index_path.exists() else {"version": 1, "assets": []}
    if index["version"] != 1:
        raise ValueError("Unsupported asset index version")
    entries = {entry["id"]: entry for entry in index["assets"]}
    for key in ids:
        report = export_editor(map_name, output_dir / key / "model.glb", asset_id=key)
        descriptor = report["asset"]
        px, py, pz = descriptor["source_origin_scene"]
        sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
        descriptor["source_origin_game"] = [px, -py * sin, pz * cos]
        for part in descriptor["parts"]:
            obstacle = json.loads(json.dumps(level["sight_obstacles"][part["source_obstacle"]]))
            for point in obstacle["points"]:
                point["x"] -= px
                point["y"] += py * sin
                point["z_bottom"] -= pz * cos
                point["z_top"] -= pz * cos
            part["obstacle_local_game"] = obstacle
        (output_dir / key / "asset.json").write_text(json.dumps(descriptor, indent=2) + "\n")
        entries[key] = {"id": key, "name": descriptor["name"], "source_map": map_name,
                        "descriptor": key + "/asset.json", "model": key + "/model.glb"}
    index["assets"] = sorted(entries.values(), key=lambda entry: entry["id"])
    index_path.write_text(json.dumps(index, indent=2) + "\n")
    return {"assets": len(ids), "index": str(index_path)}
