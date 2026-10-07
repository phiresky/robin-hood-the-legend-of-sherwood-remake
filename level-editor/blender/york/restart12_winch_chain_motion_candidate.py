"""Insert a complete moving chain while guarding every frozen timber component."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'winch-components-source-v2';CHAIN=WORK/'winch-chain-loop-prototype-v5';PLAN=WORK/'winch-closed-loop-fit-v5/motion-plan.json';OUT=WORK/'winch-complete-chain-motion-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector,Matrix
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
freeze=json.loads((BASE/'component-freeze.json').read_text());plan=json.loads(PLAN.read_text());proposal=json.loads((CHAIN/'proposal.json').read_text());assert sha(BASE/'model.blend')==freeze['model_sha256'];assert sha(CHAIN/'model.blend')==plan['model_sha256']==proposal['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene
old=[o for o in scene.objects if o.name.startswith('Suspended chain link')];assert len(old)==68
protected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in old}
for o in old:bpy.data.objects.remove(o,do_unlink=True)
with bpy.data.libraries.load(str(CHAIN/'model.blend'),link=False) as (available,loaded):loaded.objects=[name for name in available.objects if name.startswith('Complete chain loop link') or name.startswith('Inferred upper')]
for o in loaded.objects:scene.collection.objects.link(o)
links=sorted((o for o in loaded.objects if o.name.startswith('Complete chain loop link')),key=lambda o:o.name);assert len(links)==64
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));up=Vector((0,0,1))
def world(x,y,z):return Vector((x,-y/s,z/c))
left=world(2400.5,1059,104);right=world(2410,1064,104);mid=(left+right)/2;direction=(right-left).normalized();radius=proposal['radius_world'];height=proposal['straight_height'];length=proposal['path_length'];spacing=proposal['spacing_world'];recipe=Path(__file__).with_name('restart11_winch_closed_loop_fit.py');node=next(n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef) and n.name=='pose');exec(compile(ast.Module(body=[node],type_ignores=[]),str(recipe),'exec'))
for row in plan['chosen']['rows']:
 scene.frame_set(row['tick'])
 for i,o in enumerate(links):
  o.location,o.rotation_euler=pose(i*spacing+(5.5+row['unwrapped_phase_game'])/c,i);o.keyframe_insert('location',frame=row['tick']);o.keyframe_insert('rotation_euler',frame=row['tick'])
 bpy.context.view_layer.update();assert freeze['poses'][row['source_frame']]['components']=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in freeze['scope']}
for o in links:
 o.keyframe_insert('location',frame=90);o.keyframe_insert('rotation_euler',frame=90)
 for layer in o.animation_data.action.layers:
  for strip in layer.strips:
   bag=strip.channelbag(o.animation_data.action_slot)
   if bag:
    for curve in bag.fcurves:
     for key in curve.keyframe_points:key.interpolation='CONSTANT'
scene.frame_set(88);bpy.context.view_layer.update();assert protected=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in protected};OUT.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'motion.json').write_text(json.dumps({'status':'Private complete-chain motion candidate; saved pose/contact/source review pending, no approval','model_sha256':sha(OUT/'model.blend'),'stable_component_model_sha256':freeze['model_sha256'],'stable_component_freeze_sha256':sha(BASE/'component-freeze.json'),'chain_model_sha256':proposal['model_sha256'],'plan_sha256':sha(PLAN),'pose_function_source_sha256':sha(recipe),'all24_components_exact_in_all45_poses':True,'outside_geometry_appearance_exact':len(protected),'rows':plan['chosen']['rows'],'limitations':['Complete hidden return, idler bearing attachment and moving-part attachment remain inferred and unresolved.','Independent native link phases use descending-part motion only as a smoothness prior, not a fixed drive ratio.','Four source hole conflicts remain across frames25/30/31; no perfect source silhouette claim.','Continuous interpolation is not used; native90tick frame-hold timing retained.','Chain still has diagnostic material; no texture synthesis or canonical integration.']},indent=2)+'\n');print('COMPLETE CHAIN MOTION PRIVATE CANDIDATE SAVED')
