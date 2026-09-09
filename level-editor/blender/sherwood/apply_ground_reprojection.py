"""Install the synthesized clean ground and refresh its projection UVs via MCP."""
import json
import math
import hashlib
from pathlib import Path
import bpy

OUT=Path(bpy.data.filepath).parent/'ground-reprojection'
report=json.loads((OUT/'fill-validation.json').read_text())
if report['outside_mask_changed_pixels']!=0:
    raise RuntimeError('Ground fill changed pixels outside the audited mask')
obj=bpy.data.objects['Terrain - lightly undulating clearing']
baseline=bpy.data.objects['ground']
if obj.data==baseline.data:
    raise RuntimeError('Refined ground unexpectedly shares the original baseline mesh')
old=obj.data.materials[0]
name='Ground - refreshed ownership and texture synthesis'
refresh=old.name==name
if name in bpy.data.materials and not refresh:
    raise RuntimeError('Clean ground material exists but is not assigned to this ground')
scene=bpy.data.scenes['Sherwood Fast Turntable']
bpy.context.window.scene=scene
scene.render.engine='BLENDER_EEVEE'
scene.view_layers[0].material_override=None
for item in scene.objects:
    if item.get('turntable_ambient'):
        item.hide_render=False
if not refresh:
    for frame in [1,51,151,251]:
        scene.frame_set(frame)
        scene.render.filepath=str(OUT/f'before-{frame:03}.png')
        bpy.ops.render.render(write_still=True)
material=old if refresh else old.copy()
material.name=name
textures=[n for n in material.node_tree.nodes if n.type=='TEX_IMAGE']
if len(textures)!=1:
    raise RuntimeError('Expected one source texture on ground material')
image=bpy.data.images.load(str(OUT/'ground-clean.png'),check_existing=False)
image.name='Sherwood - synthesized clean ground'
image.pack()
if hashlib.sha256(bytes(image.packed_file.data)).digest()!=hashlib.sha256((OUT/'ground-clean.png').read_bytes()).digest():
    raise RuntimeError('Packed ground image does not match the synthesized file')
textures[0].image=image
textures[0].interpolation='Closest'
old.use_fake_user=True
obj.data.materials[0]=material
sin=math.sin(math.radians(35));cos=math.cos(math.radians(35))
uv=obj.data.uv_layers['Original map projection']
max_shift=0.
for loop in obj.data.loops:
    point=obj.matrix_world@obj.data.vertices[loop.vertex_index].co
    coordinates=(point.x/1920,1-(-point.y*sin-point.z*cos)/1088)
    max_shift=max(max_shift,abs(uv.data[loop.index].uv.x-coordinates[0]),abs(uv.data[loop.index].uv.y-coordinates[1]))
    uv.data[loop.index].uv=coordinates
obj['ground_reprojection']='Current static mesh silhouettes + audited Day-map trunk/root bounds; texture-synthesis CLI fill'
obj['ground_fill_report']=str(OUT/'fill-validation.json')
obj['ground_source_limit']='Ground hidden by trees/structures is inferred from audited source-ground pixels'
for frame in [1,51,151,251]:
    scene.frame_set(frame)
    scene.render.filepath=str(OUT/f'after-{frame:03}.png')
    bpy.ops.render.render(write_still=True)
scene.frame_set(1)
bpy.ops.file.pack_all()
bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
result={'material':material.name,'image':image.name,'refreshed_uv_loops':len(obj.data.loops),
        'max_uv_shift':max_shift,'baseline_material':baseline.data.materials[0].name,'packed_matches_file':True,'views':[1,51,151,251]}
(OUT/'blender-validation.json').write_text(json.dumps(result,indent=2))
