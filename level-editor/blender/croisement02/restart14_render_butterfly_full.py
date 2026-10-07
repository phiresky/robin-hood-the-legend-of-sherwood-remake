"""Save one hinged butterfly rig with a conserved wing pattern and native clock."""
from pathlib import Path
import sys,json,math,hashlib,shutil
import bpy,numpy as np
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart14_render_butterfly_rig import B,SIN,COS,rot
ROOT=HERE.parents[2];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
VERSION=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'rig-full-v1'
OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'/VERSION
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def signature(mesh):return hashlib.sha256(json.dumps({'v':[list(v.co) for v in mesh.vertices],'f':[list(p.vertices) for p in mesh.polygons]},separators=(',',':')).encode()).hexdigest()
def constant(action):
 for layer in action.layers:
  for strip in layer.strips:
   for bag in strip.channelbags:
    for fc in bag.fcurves:
     for key in fc.keyframe_points:key.interpolation='CONSTANT'
def main(*, min_free_gib=25, review_phases=(0,2,18), output_budget_mib=None, comparison_phases=None, render_threads=4, checkpoint_frames=False):
 def budget():
  assert shutil.disk_usage(OUT).free>min_free_gib*1024**3, 'Free space below scoped render floor'
  if output_budget_mib is not None:assert sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file())<output_budget_mib*1024**2, 'Scoped output budget exceeded'
 budget()
 shutil.copyfile(__file__,OUT/'executed-recipe.py');packet=json.loads((OUT/'fit.json').read_text());src=OUT.parent/'rig-v1/model.blend';source_hash=sha(src);assert source_hash=='c76fb392e7ac46dc51fec5fce9afc9058a0ad6a1b519929191b0aa236e972a11'
 bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.engine='CYCLES';s.cycles.samples=6;s.cycles.use_denoising=False;s.render.threads_mode='FIXED';s.render.threads=render_threads;s.render.resolution_x=192;s.render.resolution_y=192;s.render.resolution_percentage=100;s.render.film_transparent=True;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.fps=25;s.frame_start=1;s.frame_end=198;s.view_settings.view_transform='Standard';s.view_settings.look='None';s.world=bpy.data.worlds.new('Review');s.world.use_nodes=True;s.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
 with bpy.data.libraries.load(str(src),link=False) as (data,loaded):loaded.meshes=['body','left','right']
 meshes={m.name:m for m in loaded.meshes};rest={name:signature(m) for name,m in meshes.items()};authority=packet['material_authority'];imagepath=Path(authority['fixed_pattern_image']);assert sha(imagepath)==authority['fixed_pattern_sha256'];image=bpy.data.images.load(str(imagepath));image.pack();mat=bpy.data.materials.new('Conserved own-source wing pattern');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='EXTEND';emit=n.new('ShaderNodeEmission');output=n.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(tex.outputs['Color'],emit.inputs['Color']);mat.node_tree.links.new(emit.outputs[0],output.inputs[0])
 root=bpy.data.objects.new('Butterfly01 conserved animated rig',None);s.collection.objects.link(root);root.rotation_mode='QUATERNION';objects={};c=authority['canonical_pose_parameters'];cg=rot('z',c[2])@rot('y',c[1])@rot('x',c[0]);center=np.array(authority['canonical_source_center'])+np.array(c[5:7]);w,h=authority['canonical_source_bbox'][2:]
 for name,mesh in meshes.items():
  ob=bpy.data.objects.new(name,mesh);s.collection.objects.link(ob);ob.parent=root;ob.rotation_mode='QUATERNION';objects[name]=ob;mesh.materials.clear();mesh.materials.append(mat);layer=mesh.uv_layers.new(name='Conserved anatomical source phase2')
  for loop in mesh.loops:
   v=B.T@np.array(mesh.vertices[loop.vertex_index].co)
   if name!='body':sign=-1 if name=='left' else 1;angle=c[3] if name=='left' else c[4];v=rot('y',-sign*angle)@v+np.array([sign*.30,0,0])
   xy=(cg@v)[:2]+center;layer.data[loop.index].uv=(xy[0]/w,1-xy[1]/h)
  if name!='body':sign=-1 if name=='left' else 1;ob.location=Vector(B@np.array([sign*.30,0,0]))
 for row in packet['poses']:
  phase=row['phase'];frame=phase*2+1;rx,ry,rz,left,right,dx,dy=row['parameters'];g=rot('z',rz)@rot('y',ry)@rot('x',rx);root.rotation_quaternion=Matrix((B@g@B.T).tolist()).to_quaternion();sx=row['source']['bbox'][0]+row['source_center'][0]+dx;sy=row['source']['bbox'][1]+row['source_center'][1]+dy;z=row['inferred_altitude'];root.location=(sx,-(sy+COS*z)/SIN,z);root.keyframe_insert(data_path='location',frame=frame);root.keyframe_insert(data_path='rotation_quaternion',frame=frame)
  for name,sign,angle in [('left',-1,left),('right',1,right)]:ob=objects[name];ob.rotation_quaternion=Matrix((B@rot('y',-sign*angle)@B.T).tolist()).to_quaternion();ob.keyframe_insert(data_path='rotation_quaternion',frame=frame)
 for ob in [root,objects['left'],objects['right']]:constant(ob.animation_data.action)
 camdata=bpy.data.cameras.new('Native-first camera');camdata.type='ORTHO';camdata.ortho_scale=24;cam=bpy.data.objects.new(camdata.name,camdata);s.collection.objects.link(cam);s.camera=cam
 lights=[]
 for name,offset,energy in [('key',(-25,-35,50),22000),('fill',(35,10,20),9000)]:
  data=bpy.data.lights.new(name,'AREA');data.energy=energy;data.size=25;ob=bpy.data.objects.new(name,data);s.collection.objects.link(ob);lights.append((ob,Vector(offset)))
 expected_uv={name:hashlib.sha256(np.array([list(u.uv) for u in ob.data.uv_layers.active.data],dtype='<f4').tobytes()).hexdigest() for name,ob in objects.items()};image_name=image.name
 s.frame_set(1);bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));s=bpy.context.scene;root=bpy.data.objects['Butterfly01 conserved animated rig'];objects={name:bpy.data.objects[name] for name in meshes};cam=s.camera;lights=[(bpy.data.objects['key'],Vector((-25,-35,50))),(bpy.data.objects['fill'],Vector((35,10,20)))]
 reopened_image=bpy.data.images[image_name];assert reopened_image.packed_file is not None;packed_image_sha=hashlib.sha256(bytes(reopened_image.packed_file.data)).hexdigest();assert packed_image_sha==authority['fixed_pattern_sha256'];assert all(hashlib.sha256(np.array([list(u.uv) for u in ob.data.uv_layers.active.data],dtype='<f4').tobytes()).hexdigest()==expected_uv[name] for name,ob in objects.items())
 progress={'status':'RENDERING','model_sha256':sha(OUT/'model.blend'),'fit_sha256':sha(OUT/'fit.json'),'completed_frames':{}}
 solid=bpy.data.materials.new('Physical solid inspection');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.40,.38,.30,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.8
 def render(phase,view,path,mode='actual'):
  budget();s.frame_set(phase*2+1);center=root.location.copy();angle=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler()
  for light,offset in lights:light.location=center+offset;light.rotation_euler=(-offset).to_track_quat('-Z','Y').to_euler()
  s.view_layers[0].material_override=solid if mode=='solid' else None;bpy.context.view_layer.update();s.render.filepath=str(path);bpy.ops.render.render(write_still=True)
  if checkpoint_frames:
   progress['completed_frames'][str(path.relative_to(OUT))]={'phase':phase,'view':view,'mode':mode,'sha256':sha(path)};(OUT/'frame-checkpoints.json').write_text(json.dumps(progress,indent=2)+'\n')
 for row in packet['poses']:
  folder=OUT/'motion';folder.mkdir(exist_ok=True)
  for view in [0,4]:render(row['phase'],view,folder/f"phase-{row['phase']:02d}-view-{view}.png")
 for phase in review_phases:
  for mode in ['actual','solid']:
   folder=OUT/f'phase-{phase:02d}'/mode;folder.mkdir(parents=True,exist_ok=True)
   for view in range(8):render(phase,view,folder/f'view-{view}.png',mode)
   sheet=Image.new('RGB',(768,384),'#252525')
   for view in range(8):im=Image.open(folder/f'view-{view}.png').convert('RGBA');sheet.paste(im,(view%4*192,view//4*192),im)
   sheet.save(folder/'sheet.png')
 animation=[]
 for row in packet['poses']:
  phase=row['phase'];sheet=Image.new('RGB',(576,216),'#252525');draw=ImageDraw.Draw(sheet);original=Image.open(row['source']['source']).convert('RGBA');original=original.resize((original.width*8,original.height*8),Image.Resampling.NEAREST);sheet.paste(original,((192-original.width)//2,24+(192-original.height)//2),original)
  for col,view in [(1,0),(2,4)]:im=Image.open(OUT/'motion'/f'phase-{phase:02d}-view-{view}.png').convert('RGBA');sheet.paste(im,(col*192,24),im)
  for col,label in enumerate(['Native source','Conserved rig: native','Conserved rig: rear']):draw.text((col*192+4,5),label,fill='white')
  draw.text((5,202),f'Phase{phase:02d} ticks{phase*2:03d}-{phase*2+1:03d}',fill='white');animation.append(sheet)
 animation[0].save(OUT/'source-native-rear.gif',save_all=True,append_images=animation[1:],duration=80,loop=0,disposal=2)
 selected=list(comparison_phases) if comparison_phases is not None else [0,2,8,18,29,37,53,75,98];sheet=Image.new('RGB',(576,216*len(selected)),'#252525')
 for i,phase in enumerate(selected):sheet.paste(animation[phase],(0,i*216))
 sheet.save(OUT/'nine-phase-comparison.png')
 budget();checks=[]
 for row in packet['poses']:
  s.frame_set(row['phase']*2+1);rx,ry,rz,left,right,dx,dy=row['parameters'];g=rot('z',rz)@rot('y',ry)@rot('x',rx);expected_q=Matrix((B@g@B.T).tolist()).to_quaternion();assert abs(root.rotation_quaternion.dot(expected_q))>1-1e-6;sx=row['source']['bbox'][0]+row['source_center'][0]+dx;sy=row['source']['bbox'][1]+row['source_center'][1]+dy;z=row['inferred_altitude'];assert (root.location-Vector((sx,-(sy+COS*z)/SIN,z))).length<.002
  for name,sign,angle in [('left',-1,left),('right',1,right)]:assert abs(objects[name].rotation_quaternion.dot(Matrix((B@rot('y',-sign*angle)@B.T).tolist()).to_quaternion()))>1-1e-6
  q0=list(root.rotation_quaternion);loc0=list(root.location);a0={name:list(objects[name].rotation_quaternion) for name in ['left','right']};s.frame_set(row['phase']*2+2);assert q0==list(root.rotation_quaternion) and loc0==list(root.location) and all(a0[name]==list(objects[name].rotation_quaternion) for name in a0);checks.append(row['phase'])
 assert all(signature(objects[name].data)==rest[name] for name in rest);assert sha(src)==source_hash
 guard={'model_sha256':sha(OUT/'model.blend'),'rest_geometry_exact':rest,'source_parent_unchanged':True,'saved99_two_tick_holds_exact':len(checks)==99,'single_rig_meshes':len([o for o in s.objects if o.type=='MESH']),'fixed_material_image_sha256':authority['fixed_pattern_sha256'],'reopened_packed_image_sha256':packed_image_sha,'reopened_uv_matches_pre_save':True,'saved_pose_parameters_match_frozen_fit':True,'fixed_uv_maps':{name:hashlib.sha256(np.array([list(u.uv) for u in ob.data.uv_layers.active.data],dtype='<f4').tobytes()).hexdigest() for name,ob in objects.items()},'no_per_phase_meshes_or_uv_animation':True,'source_cycle_ticks':198,'original_camera_first':True,'full_native_render_parity_claim':False,'review_phases':list(review_phases),'render_threads':render_threads,'disk_floor_gib':min_free_gib,'output_budget_mib':output_budget_mib,'fit_summary':json.loads((OUT/'fit-summary.json').read_text())};assert guard['single_rig_meshes']==3;(OUT/'validation.json').write_text(json.dumps(guard,indent=2)+'\n');print('FULL_RIG_READY',guard['model_sha256'])
 if checkpoint_frames:
  assert len(progress['completed_frames'])==198+16*len(review_phases);progress['status']='SAVED_MODEL_GUARDS_PASS_PENDING_VISUAL_REVIEW';progress['validation_sha256']=sha(OUT/'validation.json');(OUT/'frame-checkpoints.json').write_text(json.dumps(progress,indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
