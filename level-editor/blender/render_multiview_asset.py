"""Render the current asset with the exact cameras used for texture generation."""
import json
from pathlib import Path
import bpy
from mathutils import Matrix
import render_views


def render(manifest_path, output_dir, modes=("textured",), width=384):
    manifest=json.loads(Path(manifest_path).read_text())
    scene=bpy.data.scenes['Derby Refinement']
    hidden=[(o,o.hide_render) for o in scene.objects if o.type=='MESH']
    previous_size=(scene.render.resolution_x,scene.render.resolution_y)
    cameras=[];views={}
    try:
        for obj,value in hidden:
            obj.hide_render=value or obj.get('asset_group')!=manifest['asset_id']
        crop=manifest['views'][0]['crop']
        scene.render.resolution_x=crop['width'];scene.render.resolution_y=crop['height']
        for view in manifest['views']:
            data=bpy.data.cameras.new('Multiview review '+str(view['index']))
            data.type='ORTHO';data.ortho_scale=view['ortho_scale'];data.clip_end=10000
            camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera)
            camera.matrix_world=Matrix(view['camera_matrix_world']);cameras.append(camera)
            views['view-'+str(view['index'])]=camera.name
        return render_views.render_views(scene.name,views,output_dir,modes=modes,width=width)
    finally:
        for obj,value in hidden:obj.hide_render=value
        scene.render.resolution_x,scene.render.resolution_y=previous_size
        for camera in cameras:
            data=camera.data;bpy.data.objects.remove(camera,do_unlink=True);bpy.data.cameras.remove(data)
