"""Import a volume export into an isolated refinement scene through Blender MCP."""
import json
import math
from pathlib import Path

import bpy
from mathutils import Vector


def fit_camera(camera, objects, aspect, *, points=None, padding=1.08):
    """Fit complete world-space bounds to an orthographic inspection camera."""
    rotation = camera.rotation_euler.to_matrix()
    right, up = rotation.col[0], rotation.col[1]
    points = list(points) if points is not None else [obj.matrix_world @ Vector(corner) for obj in objects if obj.type == "MESH" for corner in obj.bound_box]
    if not points or aspect <= 0 or padding < 1:
        raise ValueError("Camera fitting requires mesh bounds and a positive aspect ratio")
    xs, ys = [point.dot(right) for point in points], [point.dot(up) for point in points]
    camera.location += right * ((min(xs) + max(xs)) / 2 - camera.location.dot(right))
    camera.location += up * ((min(ys) + max(ys)) / 2 - camera.location.dot(up))
    height = max(max(ys) - min(ys), (max(xs) - min(xs)) / aspect) * padding
    camera.data.ortho_scale = height * max(1, aspect)
    backward = rotation.col[2]
    depths = [point.dot(backward) for point in points]
    distance = max(camera.location.dot(backward), max(depths) + max(height, 10))
    camera.location += backward * (distance - camera.location.dot(backward))
    camera.data.clip_start = 0.1
    camera.data.clip_end = max(camera.data.clip_end, distance - min(depths) + max(height, 10))


def setup_map(metadata_path, output_path):
    metadata_path = Path(metadata_path).resolve()
    metadata = json.loads(metadata_path.read_text())
    name = metadata["map"] + " Refinement"
    if name in bpy.data.scenes or Path(output_path).exists():
        raise FileExistsError("Refinement already exists; open its checkpoint")
    if metadata["camera"]["kind"] != "oblique-orthographic":
        raise ValueError("Expected an oblique orthographic map")
    w, h = metadata["size"]
    elevation = metadata["camera"]["elevation_deg"]
    scene = bpy.data.scenes.new(name)
    bpy.context.window.scene = scene
    bpy.ops.import_scene.gltf(filepath=str(metadata_path.with_suffix(".glb")))
    baseline = bpy.data.collections.new(metadata["map"] + " Baseline")
    scene.collection.children.link(baseline)
    for obj in list(scene.objects):
        baseline.objects.link(obj)
        for collection in list(obj.users_collection):
            if collection != baseline:
                collection.objects.unlink(obj)
    working = bpy.data.collections.new(metadata["map"] + " Working")
    scene.collection.children.link(working)
    copies = {}
    for obj in baseline.objects:
        copy = obj.copy()
        if obj.data:
            copy.data = obj.data.copy()
        working.objects.link(copy)
        copy["source_obstacle"] = obj.name
        copies[obj] = copy
    for obj, copy in copies.items():
        if obj.parent:
            copy.parent = copies[obj.parent]
    baseline.hide_render = True
    baseline.hide_viewport = True
    views = bpy.data.collections.new(metadata["map"] + " Inspection")
    scene.collection.children.link(views)
    target = Vector((w / 2, -h / (2 * math.sin(math.radians(elevation))), 0))
    camera_names = {}
    # Ortho scale is the larger image dimension with AUTO sensor fit.
    for label, yaw, pitch, span in (
        ("reference", 0, elevation, max(w, h)),
        ("east", 40, 45, h / math.sin(math.radians(elevation)) * 1.1),
        ("west", -40, 45, h / math.sin(math.radians(elevation)) * 1.1),
        ("plan", 0, 90, h / math.sin(math.radians(elevation)) * 1.1),
    ):
        data = bpy.data.cameras.new(metadata["map"] + " " + label)
        camera = bpy.data.objects.new(data.name, data)
        views.objects.link(camera)
        yaw, pitch = math.radians(yaw), math.radians(pitch)
        camera.location = target + Vector((math.sin(yaw) * math.cos(pitch), -math.cos(yaw) * math.cos(pitch), math.sin(pitch))) * 10000
        camera.rotation_euler = (target - camera.location).to_track_quat("-Z", "Y").to_euler()
        data.type = "ORTHO"
        data.ortho_scale = span
        data.clip_end = 30000
        camera_names[label] = camera.name
    scene.camera = bpy.data.objects[camera_names["reference"]]
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = w
    scene.render.resolution_y = h
    scene.render.resolution_percentage = 100
    bpy.context.view_layer.update()
    for label in ("east", "west", "plan"):
        fit_camera(bpy.data.objects[camera_names[label]], working.objects, w / h)
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "SINGLE"
    scene.display.shading.show_cavity = True
    for area in bpy.context.screen.areas:
        if area.type == "VIEW_3D":
            area.spaces.active.clip_end = 30000
            area.spaces.active.region_3d.view_perspective = "CAMERA"
            area.spaces.active.shading.type = "MATERIAL"
    Path(output_path).parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(output_path))
    return {"scene": scene.name, "cameras": camera_names, "file": str(output_path)}
