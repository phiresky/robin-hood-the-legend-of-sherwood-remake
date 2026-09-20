"""Repeatable isolated source/orbit inspection for one complete logical asset."""
import runpy
from pathlib import Path


def inspect_asset(map_name, asset_id, output_dir, width=900):
    import bpy
    directory = Path(__file__).parent
    scene = bpy.data.scenes[map_name + " Refinement"]
    working = bpy.data.collections[map_name + " Working"]
    meshes = [o for o in working.all_objects if o.type == "MESH"]
    selected = [o for o in meshes if o.get("asset_group") == asset_id and not o.hide_render]
    if not selected:
        raise ValueError(f"No visible components for {asset_id}")
    fit = runpy.run_path(str(directory / "setup_map.py"))["fit_camera"]
    render = runpy.run_path(str(directory / "render_views.py"))["render_views"]
    visibility = [(o, o.hide_render) for o in meshes]
    old_scene = bpy.context.window.scene
    cameras = []
    try:
        bpy.context.window.scene = scene
        for obj, hidden in visibility:
            obj.hide_render = hidden or obj.get("asset_group") != asset_id
        bpy.context.view_layer.update()
        views = {}
        for angle in ("reference", "east", "west"):
            template = bpy.data.objects[map_name + " " + angle]
            camera = template.copy()
            camera.data = template.data.copy()
            scene.collection.objects.link(camera)
            cameras.append(camera)
            fit(camera, selected, scene.render.resolution_x / scene.render.resolution_y)
            views[angle] = camera.name
        return render(scene.name, views, output_dir, width=width)
    finally:
        for obj, hidden in visibility:
            obj.hide_render = hidden
        for camera in cameras:
            data = camera.data
            bpy.data.objects.remove(camera, do_unlink=True)
            bpy.data.cameras.remove(data)
        bpy.context.window.scene = old_scene
        bpy.context.view_layer.update()
