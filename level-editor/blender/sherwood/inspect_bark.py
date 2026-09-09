"""Render matched central-oak views before/after refine_bark.py via Blender MCP.

Set LABEL to 'before' or 'after' in the execution scope.
"""
import math
from pathlib import Path
import bpy
from mathutils import Vector

label = globals()['LABEL']
out = Path(bpy.data.filepath).parent/'bark-inspection'
out.mkdir(exist_ok=True)
name = 'Sherwood Bark Inspection'
if name not in bpy.data.scenes:
    scene = bpy.data.scenes['Sherwood Refinement'].copy()
    scene.name = name
    scene.timeline_markers.clear()
    camera = bpy.data.objects.new('Bark inspection camera', bpy.data.cameras.new('Bark inspection lens'))
    scene.collection.objects.link(camera)
    scene.camera = camera
else:
    scene = bpy.data.scenes[name]
camera = scene.camera
camera.data.type = 'ORTHO'
camera.data.ortho_scale = 560
camera.data.clip_end = 5000
# EEVEE matches the emission materials used by the turntable.
scene.render.engine = 'BLENDER_EEVEE'
scene.eevee.taa_render_samples = 8
scene.render.resolution_x = 900
scene.render.resolution_y = 1000
scene.render.resolution_percentage = 100
scene.render.use_border = False
scene.render.image_settings.file_format = 'PNG'
scene.view_layers[0].material_override = None
obj = bpy.data.objects['Tree 032 - tapered trunk']
center = sum((v.co for v in obj.data.vertices), Vector()) / len(obj.data.vertices)
center.z = 180
scene.frame_set(1)
for degrees in (0, 90, 180, 270):
    a = math.radians(degrees)
    direction = Vector((math.sin(a)*.9, -math.cos(a)*.9, .44)).normalized()
    camera.location = center + direction*1800
    camera.rotation_euler = (center-camera.location).to_track_quat('-Z', 'Y').to_euler()
    scene.render.filepath = str(out/f'{label}-{degrees:03}.png')
    bpy.ops.render.render(scene=scene.name, write_still=True)
result = {'output':str(out), 'label':label, 'views':4}
