"""Render a neutral geometry review of an unpublished doorway candidate."""
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Vector

config_path = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
config = json.loads(config_path.read_text())
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=config["outputModel"])
bpy.context.view_layer.update()
points = [obj.matrix_world @ Vector(corner) for obj in bpy.data.objects
          if obj.type == "MESH" for corner in obj.bound_box]
minimum = Vector(tuple(min(point[axis] for point in points) for axis in range(3)))
maximum = Vector(tuple(max(point[axis] for point in points) for axis in range(3)))
center = (minimum + maximum) / 2
size = max(maximum - minimum)
scene = bpy.context.scene
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 16
scene.render.resolution_x = 640
scene.render.resolution_y = 640
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = "PNG"
scene.world = bpy.data.worlds.new("Review background")
scene.world.use_nodes = True
scene.world.node_tree.nodes["Background"].inputs[0].default_value = (0.12, 0.12, 0.12, 1)
scene.world.node_tree.nodes["Background"].inputs[1].default_value = 0.5
material = bpy.data.materials.new("Neutral geometry review")
material.use_nodes = True
material.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.55, 0.55, 0.55, 1)
material.node_tree.nodes["Principled BSDF"].inputs["Roughness"].default_value = 1
scene.view_layers[0].material_override = material
camera_data = bpy.data.cameras.new("Review camera")
camera = bpy.data.objects.new("Review camera", camera_data)
scene.collection.objects.link(camera)
camera_data.type = "ORTHO"
camera_data.ortho_scale = size * 1.3
camera_data.clip_end = 10000
scene.camera = camera
light_data = bpy.data.lights.new("Review light", "AREA")
light_data.energy = size * size * 30
light_data.shape = "DISK"
light_data.size = size * 2
light = bpy.data.objects.new("Review light", light_data)
scene.collection.objects.link(light)
angle = math.radians(config["camera"]["elevation_deg"])
for name, direction in [("front", Vector((0, -math.cos(angle), math.sin(angle)))),
                        ("rear", Vector((0.8, 1, 0.6)).normalized())]:
    camera.location = center + direction * size * 4
    camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
    light.location = center + (direction + Vector((-0.5, 0, 1))) * size * 2
    light.rotation_euler = (center - light.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = str(config_path.parent / f"geometry-{name}.png")
    bpy.ops.render.render(write_still=True)
