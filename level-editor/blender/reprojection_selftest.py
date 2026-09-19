"""Run with blender --background --factory-startup --python <this file>."""
import importlib.util
import math
import json
import struct
from pathlib import Path

import bpy
from mathutils import Vector

script = Path(__file__).with_name("reproject_map.py")
spec = importlib.util.spec_from_file_location("reproject_map", script)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
output = script.parent.parent / "work/reprojection-selftest"
output.mkdir(parents=True, exist_ok=True)
collection = bpy.data.collections.new("Test Working")
bpy.context.scene.collection.children.link(collection)
image = bpy.data.images.new("source", 64, 64)
image.generated_color = (0.8, 0.3, 0.2, 1)
image.filepath_raw = str(output / "source.png")
image.file_format = "PNG"
image.save()
material = bpy.data.materials.new("fallback")


def triangle(name, points, reverse=False):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(points, [], [(2, 1, 0) if reverse else (0, 1, 2)])
    mesh.materials.append(material)
    uv = mesh.uv_layers.new(name="Fallback UV")
    uv.active_render = True
    for loop in uv.data:
        loop.uv = (0.25, 0.75)
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    return obj


front = triangle("visible", [(4, -20, 0), (20, -20, 0), (4, -4, 0)])
ray = Vector((0, -math.cos(math.radians(35)), math.sin(math.radians(35))))
covered = triangle("covered", [Vector(p) - ray * 10 for p in [(4, -20, 0), (20, -20, 0), (4, -4, 0)]])
back = triangle("back", [(40, -20, 0), (60, -20, 0), (40, -4, 0)], reverse=True)
report = module.reproject_map("Test", output / "source.png", output / "report.json")
by_name = {item["object"]: item for item in report["objects"]}
assert by_name["visible"]["projected_faces"] == 1, report
assert by_name["covered"]["fallback_reasons"] == {"occluded": 1}, report
assert by_name["back"]["fallback_reasons"] == {"backfacing": 1}, report
assert front.data.uv_layers["Fallback UV"].active_render
assert all(tuple(loop.uv) == (0.25, 0.75) for loop in front.data.uv_layers["Fallback UV"].data)
first_hash = report["geometry_sha256"]
front.location.x += 25
report = module.reproject_map("Test", output / "source.png", output / "rerun.json")
by_name = {item["object"]: item for item in report["objects"]}
assert by_name["covered"]["projected_faces"] == 1, report
assert first_hash != report["geometry_sha256"]
assert len(front.data.materials) == 2
assert len(front.data.uv_layers) == 2
assert abs(front.data.uv_layers["Refreshed map projection"].data[0].uv.x - 29/64) < 1e-6

# A roof and its interior can occupy the same source pixels while sampling
# different artwork. Interior visibility must omit the roof, but still retain
# occlusion by other interior objects and preserve the original fallback UVs.
front["source_node"] = "shell"
covered["source_node"] = "furniture"
back["source_node"] = "hidden"
front.location.x = 0
revealed = bpy.data.images.new("revealed", 64, 64)
revealed.generated_color = (0.1, 0.7, 0.2, 1)
revealed.filepath_raw = str(output / "revealed.png")
revealed.file_format = "PNG"
revealed.save()
for repeat in range(2):
    module.restore_projection("Test")
    module.reproject_map("Test", output / "source.png", output / "shell.json",
                         receiver_nodes=["shell"], occluder_nodes=["shell"],
                         projection_label="exterior")
    interior_report = module.reproject_map(
        "Test", output / "revealed.png", output / "interior.json",
        receiver_nodes=["furniture", "hidden"], occluder_nodes=["furniture", "hidden"],
        projection_label="interior")
    rows = {row["source_node"]: row for row in interior_report["objects"]}
    assert rows["furniture"]["projected_faces"] == 1, interior_report
    assert rows["hidden"]["fallback_reasons"] == {"backfacing": 1}, interior_report
    assert covered.data.uv_layers["Fallback UV"].active_render
    assert len(covered.data.uv_layers) == 3
    assert len(covered.data.materials) == 3
assert len([im for im in bpy.data.images if im.get("reprojection_source_sha256")]) == 2

# Inspect the actual exported primitive: a stale TEXCOORD_0 fallback would look
# plausible in Blender while sampling the wrong source region in the editor.
for obj in bpy.context.selected_objects:
    obj.select_set(False)
covered.select_set(True)
bpy.context.view_layer.objects.active = covered
target = output / "layered.glb"
bpy.ops.export_scene.gltf(filepath=str(target), export_format="GLB", use_selection=True)
data = target.read_bytes()
size = struct.unpack_from("<I", data, 12)[0]
document = json.loads(data[20:20 + size])
primitive = document["meshes"][0]["primitives"][0]
exported_material = document["materials"][primitive["material"]]
texture = exported_material["emissiveTexture"]
texcoord = texture.get("texCoord", 0)
accessor = document["accessors"][primitive["attributes"][f"TEXCOORD_{texcoord}"]]
view = document["bufferViews"][accessor["bufferView"]]
assert accessor["componentType"] == 5126 and accessor["type"] == "VEC2"
offset = 20 + size + 8 + view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
stride = view.get("byteStride", 8)
actual = sorted(tuple(round(v, 5) for v in struct.unpack_from("<ff", data, offset + i * stride))
                for i in range(accessor["count"]))
expected = sorted((round(u, 5), round(1-v, 5)) for u, v in
                  (module.project_uv(covered.matrix_world @ point.co, 64, 64)
                   for point in covered.data.vertices))
assert actual == expected, (actual, expected, texcoord)
print("PASS: visibility, fallback UVs, repeat stability, independent cover/interior artwork, exported projection UVs")
