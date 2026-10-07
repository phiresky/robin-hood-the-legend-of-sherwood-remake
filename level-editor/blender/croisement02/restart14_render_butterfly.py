"""Save and inspect a private source-fitted butterfly with physical wing depth."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
VERSION=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'prototype-v1'
OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'/VERSION;SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 shutil.copyfile(__file__,OUT/'executed-recipe.py');packet=json.loads((OUT/'packet.json').read_text());bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=32;scene.render.threads_mode='FIXED';scene.render.threads=4
 scene.render.resolution_x=320;scene.render.resolution_y=320;scene.render.resolution_percentage=100;scene.render.film_transparent=True
 scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.world=bpy.data.worlds.new('Review world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.08,.08,.08,1)
 scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.fps=25;scene.frame_start=1;scene.frame_end=198;objects=[]
 for f in packet['frames']:
  mesh=bpy.data.meshes.new(f"papillon01 phase {f['index']:02d}");mesh.from_pydata(f['vertices'],[],f['faces']);mesh.update();ob=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(ob);ob.location=f['provisional_world_anchor'];objects.append(ob)
  uv=mesh.uv_layers.new(name='Native source UV')
  for poly,coords in zip(mesh.polygons,f['uv']):
   for li,co in zip(poly.loop_indices,coords):uv.data[li].uv=co
  image=bpy.data.images.load(f['source'],check_existing=True);image.pack();mat=bpy.data.materials.new(mesh.name+' exact source');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';emit=n.new('ShaderNodeEmission');out=n.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(tex.outputs['Color'],emit.inputs['Color']);mat.node_tree.links.new(emit.outputs[0],out.inputs[0]);mesh.materials.append(mat)
  ob['native_frame']=f['index'];ob['first_tick']=f['first_tick'];ob['duration_ticks']=f['duration_ticks'];ob['source_sha256']=f['sha256'];ob['depth_inferred']=True
  for time,hide in [(0,True),(f['first_tick']+1,False),(f['first_tick']+f['duration_ticks']+1,True)]:
   ob.hide_render=hide;ob.hide_viewport=hide;ob.keyframe_insert(data_path='hide_render',frame=time);ob.keyframe_insert(data_path='hide_viewport',frame=time)
  if ob.animation_data:
   for layer in ob.animation_data.action.layers:
    for strip in layer.strips:
     for bag in strip.channelbags:
      for fc in bag.fcurves:
       for key in fc.keyframe_points:key.interpolation='CONSTANT'
 camdata=bpy.data.cameras.new('Review camera');camdata.type='ORTHO';camdata.ortho_scale=24;cam=bpy.data.objects.new('Review camera',camdata);scene.collection.objects.link(cam);scene.camera=cam
 for name,loc,energy,size in [('key',(-30,-40,60),90000,35),('fill',(30,20,30),45000,30)]:
  ld=bpy.data.lights.new(name,'AREA');ld.energy=energy;ld.shape='DISK';ld.size=size;light=bpy.data.objects.new(name,ld);scene.collection.objects.link(light);light.location=Vector(packet['frames'][0]['provisional_world_anchor'])+Vector(loc);light.rotation_euler=(-Vector(loc)).to_track_quat('-Z','Y').to_euler()
 scene.frame_set(1);bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
 bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));scene=bpy.context.scene;cam=scene.camera;camdata=cam.data;objects=sorted([o for o in scene.objects if 'native_frame' in o],key=lambda o:o['native_frame'])
 solid=bpy.data.materials.new('Physical membrane inspection');solid.diffuse_color=(.65,.62,.48,1);solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.62,.48,1)
 cameras=[]
 def render(phase,view,path,solid_mode=False):
  scene.frame_set(phase*2+1);center=Vector(packet['frames'][phase]['provisional_world_anchor']);angle=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.view_layers[0].material_override=solid if solid_mode else None;bpy.context.view_layer.update();scene.render.filepath=str(path);bpy.ops.render.render(write_still=True)
  return {'phase':phase,'view':view,'native':view==0,'camera_matrix':[list(r) for r in cam.matrix_world],'ortho_scale':camdata.ortho_scale}
 for mode in ['actual','solid']:
  folder=OUT/mode;folder.mkdir(exist_ok=True)
  for i in range(8):cameras.append(render(2,i,folder/f'view-{i}.png',mode=='solid'))
  sheet=Image.new('RGB',(1280,640),'#252525')
  for i in range(8):im=Image.open(folder/f'view-{i}.png').convert('RGBA');sheet.paste(im,((i%4)*320,(i//4)*320),im)
  sheet.save(folder/'sheet.png')
 phases=[0,2,8,18,29,37,53,75,98];folder=OUT/'phase-review';folder.mkdir(exist_ok=True);sheet=Image.new('RGB',(9*192,3*212),'#252525');d=ImageDraw.Draw(sheet)
 for col,phase in enumerate(phases):
  f=packet['frames'][phase];src=Image.open(f['source']).convert('RGBA');scale=8;src=src.resize((src.width*scale,src.height*scale),Image.Resampling.NEAREST);sheet.paste(src,(col*192+(192-src.width)//2,30+(172-src.height)//2),src);d.text((col*192+6,6),f'Source phase {phase}',fill='white')
  for row,view in [(1,0),(2,4)]:
   render(phase,view,folder/f'phase-{phase:02d}-view-{view}.png');im=Image.open(folder/f'phase-{phase:02d}-view-{view}.png').convert('RGBA').resize((192,192));sheet.paste(im,(col*192,row*212+20),im);d.text((col*192+6,row*212+4),'Native physical' if row==1 else 'Rear physical',fill='white')
 sheet.save(folder/'source-native-rear.png')
 scene.frame_set(1);scene.view_layers[0].material_override=None
 for f in packet['frames']:
  scene.frame_set(f['first_tick']+1);assert [o['native_frame'] for o in objects if not o.hide_render]==[f['index']]
  image=next(n.image for n in objects[f['index']].data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');assert hashlib.sha256(bytes(image.packed_file.data)).hexdigest()==f['sha256']
 guards={'model_sha256':sha(OUT/'model.blend'),'saved_reopen_all99_phase_visibility_exact':True,'packed_source_all99_exact':True,'executed_recipe_sha256':sha(OUT/'executed-recipe.py'),'source_images_all_hash_exact':all(sha(Path(f['source']))==f['sha256'] for f in packet['frames']),'99_phases_198_ticks':len(objects)==99 and scene.frame_end==198,'source_projection_error_max':max(f['max_projection_error'] for f in packet['frames']),'cameras':cameras,'hypothesis':packet['hypothesis']}
 (OUT/'validation.json').write_text(json.dumps(guards,indent=2)+'\n');print('BUTTERFLY_REVIEW_READY',guards['model_sha256'])
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
