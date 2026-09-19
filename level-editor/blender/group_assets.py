"""Apply an authored asset catalog to a map's working collection through MCP."""
import json
from pathlib import Path

import bpy
from mathutils import Vector


def sync_asset_names(catalog_path):
    """Refresh catalog labels on an existing hierarchy without moving any parts."""
    catalog = json.loads(Path(catalog_path).read_text())
    working = bpy.data.collections[catalog["map"] + " Working"]
    expected = {f'building-{part["obstacle"]:03}': (group, part)
                for group in catalog["groups"] for part in group["parts"]}
    meshes = [o for o in working.objects if o.type == "MESH" and o.get("source_node") != "ground"]
    if {o.get("source_node") for o in meshes} != set(expected):
        raise ValueError("Catalog does not match existing source parts")
    for obj in meshes:
        group, _ = expected[obj["source_node"]]
        if obj.get("asset_group") != group["id"] or obj.parent.get("asset_group") != group["id"]:
            raise ValueError(f"Ownership differs from catalog: {obj.name}")
    changed = 0
    for obj in meshes:
        group, part = expected[obj["source_node"]]
        old_prefix = obj["asset_name"] + " / " + obj["part_name"]
        new_prefix = group["name"] + " / " + part["name"]
        if old_prefix != new_prefix:
            suffix = obj.name[len(old_prefix):] if obj.name.startswith(old_prefix) else ""
            obj.name = new_prefix + suffix
            obj["asset_name"], obj["part_name"] = group["name"], part["name"]
            changed += 1
        obj.parent.name = group["name"]
        obj.parent["asset_name"] = group["name"]
    return {"renamed_meshes": changed}


def group_assets(catalog_path):
    catalog = json.loads(Path(catalog_path).read_text())
    map_name = catalog["map"]
    working = bpy.data.collections[map_name + " Working"]
    scene = bpy.data.scenes[map_name + " Refinement"]
    expected = {}
    for group in catalog["groups"]:
        for part in group["parts"]:
            key = f'building-{part["obstacle"]:03}'
            if key in expected:
                raise ValueError(f"Duplicate asset ownership: {key}")
            expected[key] = (group, part)
    meshes = [o for o in working.objects if o.type == "MESH"]
    actual = {o.get("source_obstacle") for o in meshes} - {"ground"}
    if actual != set(expected):
        raise ValueError(f"Catalog coverage mismatch: {actual ^ set(expected)}")
    if any(o.get("asset_group") for o in working.objects):
        raise RuntimeError("Asset hierarchy already exists; inspect before revising")
    matrices = {o: o.matrix_world.copy() for o in meshes}
    roots = [o for o in working.objects if o.type == "EMPTY"]
    root = next(o for o in roots if o.get("source_obstacle") == "map")
    root.name = map_name + " Assets"
    groups = {}
    for group in catalog["groups"]:
        obj = bpy.data.objects.new(group["name"], None)
        working.objects.link(obj)
        obj.parent = root
        # Identity parent transforms retain the export's world-coordinate mesh
        # convention. The editor supplies pivots from obstacle footprints.
        obj["asset_group"] = group["id"]
        obj["asset_name"] = group["name"]
        obj.empty_display_type = "PLAIN_AXES"
        obj.empty_display_size = 15
        groups[group["id"]] = obj
    bpy.context.view_layer.update()
    names = []
    for obj in meshes:
        source = obj["source_obstacle"]
        obj["source_node"] = source
        if source == "ground":
            obj.parent = root
            obj.name = map_name + " Terrain"
        else:
            group, part = expected[source]
            obj.parent = groups[group["id"]]
            obj["asset_group"] = group["id"]
            obj["asset_name"] = group["name"]
            obj["part_name"] = part["name"]
            suffix = " (preserved ramp)" if obj.hide_render else ""
            if obj.get("step_count"):
                suffix = " (modeled treads)"
            obj.name = group["name"] + " / " + part["name"] + suffix
            obj.data.name = obj.name
            names.append({"object": obj.name, "group": group["id"], "source": source})
        obj.matrix_world = matrices[obj]
    for obj in roots:
        if obj != root:
            if obj.children:
                raise RuntimeError(f"Unexpected remaining children: {obj.name}")
            bpy.data.objects.remove(obj, do_unlink=True)
    bpy.context.view_layer.update()
    drift = max(abs(obj.matrix_world[r][c] - matrix[r][c]) for obj, matrix in matrices.items() for r in range(4) for c in range(4))
    if drift > 1e-5:
        raise RuntimeError(f"Reparenting moved geometry: {drift}")
    report = {"map": map_name, "assets": len(groups), "obstacles": len(expected),
              "working_meshes": len(meshes), "max_transform_drift": drift, "parts": names}
    out = Path(bpy.data.filepath).parent
    (out / "asset-hierarchy.json").write_text(json.dumps(report, indent=2) + "\n")
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    return {k: v for k, v in report.items() if k != "parts"}
