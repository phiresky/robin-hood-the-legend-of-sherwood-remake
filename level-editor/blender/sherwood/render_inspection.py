"""Texture-free solid and wireframe views for checking crown/trunk placement."""
import json
import sys
from pathlib import Path
import bpy
import bmesh
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).parent))
from modeling import SIN,COS,EYE

out=Path(bpy.data.filepath).parent/'inspection-depth'
out.mkdir(parents=True,exist_ok=True)
scene=bpy.data.scenes['Sherwood Refinement'];bpy.context.window.scene=scene
canopies=bpy.data.collections['10 Animated foliage - curved canopy shells']
ambient=bpy.data.collections['11 Authored ambient overlay references']
snapshot=out.parent/'pass2/canopy-depth-before.json'
previous=json.loads(snapshot.read_text()) if snapshot.is_file() else {}
report={'canopies':{}}
if not previous:report['comparison_note']='No previous depth snapshot; only current topology is checked.'
for obj in canopies.objects:
    old=previous.get(obj['profile'],[])
    errors=[];shifts=[]
    for a,v in zip(old,obj.data.vertices):
        b=v.co
        errors.append(max(abs(a[0]-b.x),abs((-a[1]*SIN-a[2]*COS)-(-b.y*SIN-b.z*COS))))
        shifts.append((b-Vector(a)).dot(EYE))
    bm=bmesh.new();bm.from_mesh(obj.data)
    report['canopies'][obj['profile']]={'max_projection_shift_pixels':max(errors) if errors else None,
        'mean_toward_camera_shift_world_units':sum(shifts)/len(shifts) if shifts else None,
        'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
        'zero_area_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
    bm.free()

wire=bpy.data.materials.new('INSPECTION - dark topology lines on neutral faces')
wire.use_nodes=True
nodes=wire.node_tree.nodes;nodes.clear();links=wire.node_tree.links
edge=nodes.new('ShaderNodeWireframe');edge.use_pixel_size=True;edge.inputs['Size'].default_value=.65
color=nodes.new('ShaderNodeMixRGB');color.blend_type='MIX'
color.inputs[1].default_value=(.43,.48,.53,1);color.inputs[2].default_value=(.012,.022,.032,1)
emission=nodes.new('ShaderNodeEmission');output=nodes.new('ShaderNodeOutputMaterial')
links.new(edge.outputs['Fac'],color.inputs[0]);links.new(color.outputs[0],emission.inputs['Color']);links.new(emission.outputs[0],output.inputs['Surface'])

ground=bpy.data.objects['Terrain - lightly undulating clearing']
state={'engine':scene.render.engine,'frame':scene.frame_current,'foliage':canopies.hide_render,'ground':ground.hide_render,
       'ambient':ambient.hide_render,'override':scene.view_layers[0].material_override}
files=[]
try:
    ambient.hide_render=True
    for name,frame,foliage,mode in [
        ('01-solid-east-foliage',2,True,'solid'),
        ('02-solid-west-foliage',3,True,'solid'),
        ('03-solid-plan-foliage',4,True,'solid'),
        ('04-solid-east-structure',2,False,'solid'),
        ('05-solid-west-structure',3,False,'solid'),
        ('06-solid-village-close',5,False,'solid'),
        ('07-solid-camp-close',7,False,'solid'),
        ('08-solid-river-close',8,False,'solid'),
        ('09-wire-east-structure',2,False,'wire'),
        ('10-wire-village-close',5,False,'wire'),
        ('11-wire-plan-foliage',4,True,'wire'),
    ]:
        scene.frame_set(frame);canopies.hide_render=not foliage
        ground.hide_render=mode=='wire'
        scene.render.engine='BLENDER_WORKBENCH' if mode=='solid' else 'BLENDER_EEVEE'
        scene.view_layers[0].material_override=None if mode=='solid' else wire
        scene.render.filepath=str(out/(name+'.png'));bpy.ops.render.render(write_still=True)
        files.append(scene.render.filepath)
    canopies.hide_render=False
    ground.hide_render=state['ground']
    scene.view_layers[0].material_override=None
    scene.render.engine='BLENDER_EEVEE'
    for name,frame in [('12-corrected-reference',1),('13-corrected-east-textured',2)]:
        scene.frame_set(frame);scene.render.filepath=str(out/(name+'.png'))
        bpy.ops.render.render(write_still=True);files.append(scene.render.filepath)
finally:
    scene.view_layers[0].material_override=state['override']
    scene.render.engine=state['engine'];scene.frame_set(1)
    canopies.hide_render=state['foliage'];ambient.hide_render=state['ambient']
    ground.hide_render=state['ground']
report['renders']=files
(out/'placement-validation.json').write_text(json.dumps(report,indent=2))
bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
result=report
