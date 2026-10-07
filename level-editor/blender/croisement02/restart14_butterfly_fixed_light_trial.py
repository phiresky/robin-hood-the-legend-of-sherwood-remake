"""One frozen lighting trial with a single reviewed local pose replacement."""
from pathlib import Path
import json,hashlib,shutil,sys,math,copy
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';O=B/'fixed-light-trial-v1';P=ROOT/'level-editor/work/croisement02-refinement/restart17-small-job-disk-policy.json';CAP=8*1024**2
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def budget():
 used=sum(p.stat().st_size for p in O.rglob('*')if p.is_file());assert used<CAP,'8MiB trial cap';assert shutil.disk_usage(ROOT).free>=8*1024**3+CAP-used,'8GiB plus remaining reserve'
def prepare():
 O.mkdir(exist_ok=True);f=json.loads((B/'rig-joint-trial-v1/fit.json').read_text());a=json.loads((B/'phase30-local-cpu-v1/report.json').read_text());assert sha(B/'rig-joint-trial-v1/fit.json')==a['fit_sha256'];new=copy.deepcopy(f);c=next(r for r in a['rows']if r['name']=='Candidate302');new['poses'][30]['parameters']=c['parameters'];assert all(new['poses'][i]==f['poses'][i]for i in range(99)if i!=30);assert new['source_clock']==f['source_clock']
 for r in f['poses']:assert sha(Path(r['source']['source']))==r['source']['sha256']
 (O/'fit.json').write_text(json.dumps(new,indent=2)+'\n');plan=json.loads((B/'fixed-light-trial-plan-v1/plan.json').read_text());receipt={'status':'ROOT_AUTHORIZED_ONE_PRIVATE_TRIAL','root_direction':'Select302; keep source asymmetry unresolved. One fixed lighting candidate. Vegetation25284 terminalEXIT0 releases small-job lane.','source_model_sha256':sha(B/'rig-joint-trial-v1/model.blend'),'pose_report_sha256':sha(B/'phase30-local-cpu-v1/report.json'),'fixed_parameters':plan,'disk_policy_sha256':sha(P),'disk_override':'Versioned override:8GiB plus remaining8MiB allowance, not obsolete10GiB scheduling threshold.','threads':2,'total_renders':124,'no_propagation':True};(O/'authorization-and-guard-v1.json').write_text(json.dumps(receipt,indent=2)+'\n');return new,plan

def execute():
 import bpy,numpy as np
 from mathutils import Vector,Matrix
 from PIL import Image,ImageDraw
 sys.path[:0]=[str(Path(__file__).resolve().parent),str(ROOT/'level-editor/refinement')]
 from restart14_render_butterfly_rig import B as basis,SIN,COS,rot
 from restart14_render_butterfly_full import signature
 from render_slots import acquire,release
 resume='--resume-render' in sys.argv
 assert resume==(O/'model.blend').exists(),'Resume must match saved model existence'
 acquire()
 try:
  packet,plan=prepare();budget();src=B/'rig-joint-trial-v1/model.blend';srcsha=sha(src)
  if resume:
   frozen=json.loads((B/'rig-joint-trial-v1/validation.json').read_text());mesh_before=frozen['rest_geometry_exact'];uv_before=frozen['fixed_uv_maps'];image_sha=plan['fixed_image_sha256'];bpy.ops.wm.open_mainfile(filepath=str(O/'model.blend'));s=bpy.context.scene;root=bpy.data.objects['Butterfly01 conserved animated rig'];obs={n:bpy.data.objects[n]for n in ['body','left','right']};cam=s.camera
  else:
   bpy.ops.wm.open_mainfile(filepath=str(src));s=bpy.context.scene;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_x=s.render.resolution_y=192;s.cycles.samples=6;s.render.resolution_percentage=100
   root=bpy.data.objects['Butterfly01 conserved animated rig'];obs={n:bpy.data.objects[n]for n in ['body','left','right']};mesh_before={n:signature(o.data)for n,o in obs.items()};uv_before={n:hashlib.sha256(np.array([list(v.uv)for v in o.data.uv_layers.active.data],dtype='<f4').tobytes()).hexdigest()for n,o in obs.items()}
   r=packet['poses'][30];rx,ry,rz,left,right,dx,dy=r['parameters'];s.frame_set(61);g=rot('z',rz)@rot('y',ry)@rot('x',rx);root.rotation_quaternion=Matrix((basis@g@basis.T).tolist()).to_quaternion();sx=r['source']['bbox'][0]+r['source_center'][0]+dx;sy=r['source']['bbox'][1]+r['source_center'][1]+dy;z=r['inferred_altitude'];root.location=(sx,-(sy+COS*z)/SIN,z);root.keyframe_insert(data_path='location',frame=61);root.keyframe_insert(data_path='rotation_quaternion',frame=61)
   for n,sign,angle in [('left',-1,left),('right',1,right)]:obs[n].rotation_quaternion=Matrix((basis@rot('y',-sign*angle)@basis.T).tolist()).to_quaternion();obs[n].keyframe_insert(data_path='rotation_quaternion',frame=61)
   mat=obs['body'].data.materials[0];nodes=mat.node_tree.nodes;links=mat.node_tree.links;tex=next(n for n in nodes if n.type=='TEX_IMAGE');image_sha=hashlib.sha256(bytes(tex.image.packed_file.data)).hexdigest();assert image_sha==plan['fixed_image_sha256'];emit=next(n for n in nodes if n.type=='EMISSION');geom=nodes.new('ShaderNodeNewGeometry');dot=nodes.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT';dot.inputs[1].default_value=tuple(basis@np.array(plan['light_direction_canonical']));links.new(geom.outputs['True Normal'],dot.inputs[0]);positive=nodes.new('ShaderNodeMath');positive.operation='MAXIMUM';positive.inputs[1].default_value=0;links.new(dot.outputs['Value'],positive.inputs[0]);mul=nodes.new('ShaderNodeMath');mul.operation='MULTIPLY';mul.inputs[1].default_value=plan['strength'];links.new(positive.outputs[0],mul.inputs[0]);add=nodes.new('ShaderNodeMath');add.operation='ADD';add.inputs[1].default_value=plan['ambient'];add.use_clamp=True;links.new(mul.outputs[0],add.inputs[0]);color=nodes.new('ShaderNodeMixRGB');color.blend_type='MULTIPLY';color.inputs[0].default_value=1;links.new(tex.outputs['Color'],color.inputs[1]);links.new(add.outputs[0],color.inputs[2]);links.new(color.outputs[0],emit.inputs['Color']);s.view_layers[0].material_override=None
   s.frame_set(1);budget();bpy.ops.wm.save_as_mainfile(filepath=str(O/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(O/'model.blend'));s=bpy.context.scene;root=bpy.data.objects['Butterfly01 conserved animated rig'];obs={n:bpy.data.objects[n]for n in obs};cam=s.camera
  guards=[]
  for r in packet['poses']:
   s.frame_set(r['phase']*2+1);rx,ry,rz,left,right,dx,dy=r['parameters'];g=rot('z',rz)@rot('y',ry)@rot('x',rx);assert abs(root.rotation_quaternion.dot(Matrix((basis@g@basis.T).tolist()).to_quaternion()))>1-1e-6
   sx=r['source']['bbox'][0]+r['source_center'][0]+dx;sy=r['source']['bbox'][1]+r['source_center'][1]+dy;z=r['inferred_altitude'];assert(root.location-Vector((sx,-(sy+COS*z)/SIN,z))).length<.002
   for n,sign,a in [('left',-1,left),('right',1,right)]:assert abs(obs[n].rotation_quaternion.dot(Matrix((basis@rot('y',-sign*a)@basis.T).tolist()).to_quaternion()))>1-1e-6
   before=[list(root.location),list(root.rotation_quaternion),*[list(obs[n].rotation_quaternion)for n in ['left','right']]];s.frame_set(r['phase']*2+2);after=[list(root.location),list(root.rotation_quaternion),*[list(obs[n].rotation_quaternion)for n in ['left','right']]];assert before==after;guards.append(r['phase'])
  assert all(signature(o.data)==mesh_before[n]for n,o in obs.items());assert all(hashlib.sha256(np.array([list(v.uv)for v in o.data.uv_layers.active.data],dtype='<f4').tobytes()).hexdigest()==uv_before[n]for n,o in obs.items());retex=next(n for n in obs['body'].data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');assert hashlib.sha256(bytes(retex.image.packed_file.data)).hexdigest()==image_sha
  progress={'status':'RENDERING','model_sha256':sha(O/'model.blend'),'completed':{}};rear=[0,2,18,27,28,29,30,47,98];tasks=[(p,0,O/'motion'/f'phase-{p:02d}-view-0.png')for p in range(99)]+[(p,4,O/'motion'/f'phase-{p:02d}-view-4.png')for p in rear]+[(p,v,O/f'phase-{p:02d}'/'actual'/f'view-{v}.png')for p in [18,30]for v in range(8)];assert len(tasks)==124
  for phase,view,path in tasks:
   budget();path.parent.mkdir(parents=True,exist_ok=True);s.frame_set(phase*2+1);center=root.location.copy();angle=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();s.render.filepath=str(path);bpy.context.view_layer.update();bpy.ops.render.render(write_still=True);progress['completed'][str(path.relative_to(O))]=sha(path);(O/'frame-checkpoints.json').write_text(json.dumps(progress,indent=2)+'\n')
  for phase in [18,30]:
   folder=O/f'phase-{phase:02d}'/'actual';sheet=Image.new('RGB',(768,384),'#252525')
   for view in range(8):im=Image.open(folder/f'view-{view}.png').convert('RGBA');sheet.paste(im,(view%4*192,view//4*192),im)
   sheet.save(folder/'sheet.png')
  for page in range(11):
   sheet=Image.new('RGB',(576,216*9),'#252525');draw=ImageDraw.Draw(sheet)
   for j,phase in enumerate(range(page*9,min(99,page*9+9))):
    source=Image.open(packet['poses'][phase]['source']['source']).convert('RGBA');source=source.resize((source.width*8,source.height*8),Image.Resampling.NEAREST);sheet.paste(source,((192-source.width)//2,j*216+24+(192-source.height)//2),source)
    for col,folder in [(1,B/'rig-joint-trial-v1'),(2,O)]:im=Image.open(folder/'motion'/f'phase-{phase:02d}-view-0.png').convert('RGBA');sheet.paste(im,(col*192,j*216+24),im)
    draw.text((4,j*216+4),f'{phase:02d} original | frozen baseline | fixed light',fill='white')
   sheet.save(O/f'all99-page-{page:02d}.png')
  sheet=Image.new('RGB',(576,216*9),'#252525');draw=ImageDraw.Draw(sheet)
  for j,phase in enumerate(rear):
   im=Image.open(packet['poses'][phase]['source']['source']).convert('RGBA');im=im.resize((im.width*8,im.height*8),Image.Resampling.NEAREST);sheet.paste(im,((192-im.width)//2,j*216+24+(192-im.height)//2),im)
   for col,view in [(1,0),(2,4)]:im=Image.open(O/'motion'/f'phase-{phase:02d}-view-{view}.png').convert('RGBA');sheet.paste(im,(col*192,j*216+24),im)
   draw.text((4,j*216+4),f'{phase:02d} original | native | rear',fill='white')
  sheet.save(O/'nine-phase-source-native-rear.png');assert sha(src)==srcsha;budget();validation={'status':'SAVED_GUARDS_PASS_PENDING_VISUAL_REVIEW','model_sha256':sha(O/'model.blend'),'source_model_unchanged':srcsha,'exact_image_sha256':image_sha,'exact_rest_geometry':mesh_before,'exact_uv':uv_before,'all99_two_tick_holds':guards,'only_phase30_pose_changed':True,'fixed_lighting':plan,'render_count':len(progress['completed']),'native_view_first':True};(O/'validation.json').write_text(json.dumps(validation,indent=2)+'\n');progress['status']='COMPLETE';(O/'frame-checkpoints.json').write_text(json.dumps(progress,indent=2)+'\n');print('FIXED_LIGHT_COMPLETE',validation['model_sha256'],flush=True)
 finally:release()
if __name__=='__main__':
 if '--execute'in sys.argv:execute()
 else:prepare();print('PREPARED_ONLY')
