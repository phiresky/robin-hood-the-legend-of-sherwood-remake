"""Align a private step mesh's top with its authored planar receiving surface."""
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
import bmesh
from mathutils import Vector

config_path = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
config = json.loads(config_path.read_text())
source = Path(config["inputModel"])
assert hashlib.sha256(source.read_bytes()).hexdigest() == config["modelSha256"]
output = Path(config["outputModel"])
assert not output.exists(), "Do not overwrite an existing candidate"
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(source))
bpy.context.scene.name = config["modelScene"]
bpy.context.view_layer.update()
root = bpy.data.objects[config["node"]]
angle = math.radians(config["camera"]["elevation_deg"])
sine, cosine = math.sin(angle), math.cos(angle)
targets = [Vector(point) for point in config["footprint"]]
changed = []
matched = set()
for obj in [root, *root.children_recursive]:
    if obj.type != "MESH":
        continue
    inverse = obj.matrix_world.inverted()
    for vertex in obj.data.vertices:
        world = obj.matrix_world @ vertex.co
        game = Vector((world.x, -world.y * sine, world.z * cosine))
        index = min(range(len(targets)), key=lambda i: (targets[i] - game).length)
        distance = (targets[index] - game).length
        if distance > config["maximumDisplacement"]:
            assert game.z < min(point.z for point in targets) - 1, "Unmatched top vertex"
            continue
        target = targets[index]
        vertex.co = inverse @ Vector((target.x, -target.y / sine, target.z / cosine))
        matched.add(index)
        changed.append({"object": obj.name, "vertex": vertex.index,
                        "before": list(game), "after": list(target), "distance": distance})
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    assert all(edge.is_manifold for edge in bm.edges), "Step repair must retain a closed shell"
    bm.to_mesh(obj.data)
    bm.free()
assert len(matched) == len(targets), "Every receiving-footprint corner must match the mesh"
bpy.ops.export_scene.gltf(filepath=str(output), export_format="GLB", export_extras=True)
report = {"scope": config["scope"], "published": False,
          "modelSha256": hashlib.sha256(output.read_bytes()).hexdigest(),
          "maximumDisplacement": max(item["distance"] for item in changed), "changed": changed}
(config_path.parent / "mesh-build.json").write_text(json.dumps(report))
print(json.dumps({key: value for key, value in report.items() if key != "changed"}))
