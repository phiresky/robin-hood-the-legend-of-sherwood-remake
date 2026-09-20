"""Blender fixture for tightly fitted, unclipped review cameras."""
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bpy
from mathutils import Vector
from setup_map import fit_camera

mesh = bpy.data.meshes.new("Slender diagonal asset")
points = [Vector((x + offset, x - offset, z))
          for x in (-10, 10) for offset in (-.5, .5) for z in (0, 5)]
mesh.from_pydata(points, [], [])
mesh.update()
obj = bpy.data.objects.new(mesh.name, mesh)
bpy.context.scene.collection.objects.link(obj)
bpy.context.view_layer.update()
camera = bpy.data.objects.new("Fit fixture", bpy.data.cameras.new("Fit fixture"))
bpy.context.scene.collection.objects.link(camera)
camera.data.type = "ORTHO"
camera.location = (30, 30, 20)
camera.rotation_euler = (-camera.location).to_track_quat("-Z", "Y").to_euler()
aspect = 384 / 512
fit_camera(camera, [obj], aspect)
old_scale = camera.data.ortho_scale
fit_camera(camera, [obj], aspect, points=points, padding=1.04)
assert camera.data.ortho_scale < old_scale * .9
rotation = camera.rotation_euler.to_matrix()
xs = [(p-camera.location).dot(rotation.col[0]) for p in points]
ys = [(p-camera.location).dot(rotation.col[1]) for p in points]
half_height = camera.data.ortho_scale / 2
assert max(abs(x) for x in xs) <= half_height * aspect
assert max(abs(y) for y in ys) <= half_height
occupancy = max(max(abs(x) for x in xs)/(half_height*aspect),
                max(abs(y) for y in ys)/half_height)
assert math.isclose(occupancy, 1/1.04, rel_tol=1e-5)
print(f"PASS: all vertices fit; limiting-axis occupancy {occupancy:.1%}; scale {old_scale:.2f} -> {camera.data.ortho_scale:.2f}")
