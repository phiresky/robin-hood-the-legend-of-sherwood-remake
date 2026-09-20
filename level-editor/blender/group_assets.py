"""Apply an authored asset catalog to a map's working collection through MCP."""
import json
from pathlib import Path

import bpy
from mathutils import Vector


def _renamed_part(obj, group, part):
    """Replace the catalog prefix while retaining authored component labels."""
    old_prefix = obj.get("asset_name", "") + " / " + obj.get("part_name", "")
    new_prefix = group["name"] + " / " + part["name"]
    if obj.name.startswith(old_prefix):
        suffix = obj.name[len(old_prefix):]
    elif obj.name.startswith(new_prefix):
        suffix = obj.name[len(new_prefix):]
    else:
        old_group = obj.get("asset_name", "") + " / "
        detail = obj.name[len(old_group):] if obj.name.startswith(old_group) else obj.name
        suffix = " / " + detail
    return new_prefix + suffix


def reconcile_asset_groups(catalog_path):
    """Apply revised ownership by stable source_node without moving geometry.

    Includes hidden retained originals and every refined mesh component sharing
    a canonical part. Creates new logical groups and removes only obsolete,
    empty asset-group objects. No blend save or publication is implicit.
    """
    catalog = json.loads(Path(catalog_path).read_text())
    working = bpy.data.collections[catalog["map"] + " Working"]
    scene = bpy.data.scenes[catalog["map"] + " Refinement"]
    previous_scene = bpy.context.window.scene
    groups, expected = {}, {}
    for group in catalog["groups"]:
        if not group["id"] or not group["name"] or group["id"] in groups or not group["parts"]:
            raise ValueError("Catalog groups must be named, unique and nonempty")
        groups[group["id"]] = group
        for part in group["parts"]:
            key = f'building-{part["obstacle"]:03}'
            if key in expected or not part["name"]:
                raise ValueError(f"Duplicate or unnamed asset part: {key}")
            expected[key] = (group, part)
    objects = list(working.all_objects)
    meshes = [obj for obj in objects if obj.type == "MESH" and obj.get("source_node") != "ground"]
    actual = {obj.get("source_node") for obj in meshes}
    if actual != set(expected):
        raise ValueError(f"Catalog coverage mismatch: {sorted(actual ^ set(expected), key=str)}")
    roots = [obj for obj in objects if obj.type == "EMPTY" and obj.get("source_obstacle") == "map"]
    if len(roots) != 1:
        raise ValueError("Expected one map asset root")
    root = roots[0]
    parents = {}
    for obj in objects:
        if obj.type == "EMPTY" and obj.get("asset_group"):
            identifier = obj["asset_group"]
            if identifier in parents:
                raise ValueError(f"Duplicate asset group object: {identifier}")
            parents[identifier] = obj
    obsolete = [obj for identifier, obj in parents.items() if identifier not in groups]
    for obj in obsolete:
        if any(child not in meshes for child in obj.children):
            raise ValueError(f"Obsolete group contains unclassified children: {obj.name}")
    matrices = {obj: obj.matrix_world.copy() for obj in meshes}
    visibility = {obj: (obj.hide_render, obj.hide_viewport) for obj in meshes}
    created, removed, moves, renamed = [], [], [], 0
    try:
        bpy.context.window.scene = scene
        for identifier, group in groups.items():
            if identifier not in parents:
                parent = bpy.data.objects.new(group["name"], None)
                working.objects.link(parent)
                parent.parent = root
                parent.empty_display_type = "PLAIN_AXES"
                parent.empty_display_size = 15
                parents[identifier] = parent
                created.append(identifier)
            parent = parents[identifier]
            parent.name = group["name"]
            parent["asset_group"], parent["asset_name"] = identifier, group["name"]
        bpy.context.view_layer.update()
        for obj in meshes:
            group, part = expected[obj["source_node"]]
            target = parents[group["id"]]
            if obj.parent != target or obj.get("asset_group") != group["id"]:
                moves.append({"source_node": obj["source_node"], "object": obj.name,
                              "from": obj.get("asset_group"), "to": group["id"],
                              "hidden": obj.hide_render})
            name = _renamed_part(obj, group, part)
            renamed += obj.name != name
            if obj.parent != target:
                obj.parent = target
                obj.matrix_world = matrices[obj]
            obj.name = name
            obj["asset_group"], obj["asset_name"], obj["part_name"] = group["id"], group["name"], part["name"]
        bpy.context.view_layer.update()
        drift = max((abs(obj.matrix_world[r][c] - matrix[r][c])
                     for obj, matrix in matrices.items() for r in range(4) for c in range(4)), default=0)
        if drift > 1e-5:
            raise RuntimeError(f"Reparenting moved geometry: {drift}")
        if any((obj.hide_render, obj.hide_viewport) != visibility[obj] for obj in meshes):
            raise RuntimeError("Reparenting altered component visibility")
        for obj in obsolete:
            if obj.children:
                raise RuntimeError(f"Obsolete group retains children: {obj.name}")
            removed.append(obj["asset_group"])
            bpy.data.objects.remove(obj, do_unlink=True)
        return {"map": catalog["map"], "assets": len(groups), "canonical_parts": len(expected),
                "working_meshes": len(meshes), "created_groups": created, "removed_groups": removed,
                "renamed_meshes": renamed, "moved_components": moves, "max_transform_drift": drift}
    finally:
        bpy.context.window.scene = previous_scene


def sync_asset_names(catalog_path):
    """Refresh catalog labels on an existing hierarchy without moving any parts."""
    catalog = json.loads(Path(catalog_path).read_text())
    working = bpy.data.collections[catalog["map"] + " Working"]
    expected = {f'building-{part["obstacle"]:03}': (group, part)
                for group in catalog["groups"] for part in group["parts"]}
    meshes = [o for o in working.all_objects if o.type == "MESH" and o.get("source_node") != "ground"]
    if {o.get("source_node") for o in meshes} != set(expected):
        raise ValueError("Catalog does not match existing source parts")
    for obj in meshes:
        group, _ = expected[obj["source_node"]]
        if obj.get("asset_group") != group["id"] or obj.parent is None or obj.parent.get("asset_group") != group["id"]:
            raise ValueError(f"Ownership differs from catalog: {obj.name}")
    changed = 0
    for obj in meshes:
        group, part = expected[obj["source_node"]]
        old_prefix = obj["asset_name"] + " / " + obj["part_name"]
        new_prefix = group["name"] + " / " + part["name"]
        if old_prefix != new_prefix:
            obj.name = _renamed_part(obj, group, part)
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
