"""Render original-camera ownership masks for a fresh ground texture bake.

Run through Blender MCP. Source geometry/materials are never mutated here.
Animated foliage/ambient sprites are separate assets, not Day-map ground pixels.
"""
from pathlib import Path
import bpy

OUT = Path(bpy.data.filepath).parent/'ground-reprojection'
OUT.mkdir(exist_ok=True)
name = 'Sherwood Ground Ownership'
if name in bpy.data.scenes:
    raise RuntimeError('Ownership scene already exists; inspect before replacing it')
main = bpy.data.scenes['Sherwood Refinement']
backup = Path(bpy.data.filepath).with_name('sherwood-before-ground-reprojection.blend')
if not backup.exists():
    bpy.ops.wm.save_as_mainfile(filepath=str(backup), copy=True)
ground = bpy.data.objects['Terrain - lightly undulating clearing']
material = ground.data.materials[0]
source = next(n.image for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
export = source.copy()
export.filepath_raw = str(OUT/'ground-before.png')
export.file_format = 'PNG'
export.save()
bpy.data.images.remove(export)

scene = bpy.data.scenes.new(name)
bpy.context.window.scene = scene
scene.render.engine = 'BLENDER_WORKBENCH'
scene.display.render_aa = '8'
scene.display.shading.light = 'FLAT'
scene.display.shading.color_type = 'SINGLE'
scene.display.shading.single_color = (1,1,1)
scene.display.shading.show_shadows = False
scene.display.shading.show_cavity = False
scene.render.film_transparent = True
scene.render.resolution_x = 1920
scene.render.resolution_y = 1088
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = 'PNG'
scene.render.image_settings.color_mode = 'RGBA'
scene.view_settings.view_transform = 'Standard'
scene.view_settings.look = 'None'
camera = bpy.data.objects['Reference Camera'].copy()
camera.data = camera.data.copy()
scene.collection.objects.link(camera)
scene.camera = camera
scene.frame_set(1)
counts = {}
groups = {}
for variant in ['refined','legacy']:
    group = bpy.data.collections.new('GROUND MASK '+variant)
    scene.collection.children.link(group)
    groups[variant] = group
    for collection in main.collection.children:
        if variant=='legacy':
            if not collection.name.startswith('00 '):
                continue
        elif collection.hide_render or collection.name.startswith(('00 ','02 ','07 ','10 ','11 ')):
            continue
        for original in collection.objects:
            if original.type!='MESH' or original.get('source_obstacle')=='ground' or original==ground or original.name=='ground':
                continue
            if variant=='refined' and original.hide_render:
                continue
            obj = original.copy()
            obj.name = 'GROUND MASK '+original.name
            obj.parent = None
            obj.matrix_world = original.matrix_world.copy()
            obj.hide_render = False
            obj.hide_viewport = False
            group.objects.link(obj)
    counts[variant] = len(group.objects)
for variant in groups:
    for key, group in groups.items():
        group.hide_render = key!=variant
    scene.render.filepath = str(OUT/f'{variant}-ownership.png')
    bpy.ops.render.render(write_still=True)
bpy.context.window.scene = main
result = {'output':str(OUT),'objects':counts,'ground_material':material.name}
