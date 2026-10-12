"""Saved articulated butterfly proposal with observed wing-face textures and native-first views."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Matrix,Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
FIT=ROOT/'level-editor/work/active/croisement02/restart64-butterfly07-pose-refinement-v1/fit.json'
ATLAS=ROOT/'level-editor/work/active/croisement02/restart64-butterfly07-observed-atlas-v1'
OUT=ROOT/'level-editor/work/active/croisement02/restart64-butterfly07-model-review-v1'
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));B=np.array([[1,0,0],[0,-SIN,-COS],[0,-COS,SIN]])
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def rot(axis,a):
 a=math.radians(a);c=math.cos(a);s=math.sin(a)
 return {'x':np.array([[1,0,0],[0,c,-s],[0,s,c]]),'y':np.array([[c,0,s],[0,1,0],[-s,0,c]]),'z':np.array([[c,-s,0],[s,c,0],[0,0,1]])}[axis]
def material(side):
 mat=bpy.data.materials.new(f'Observed wing {side}; gray unknown');mat.use_nodes=True;ns=mat.node_tree.nodes;ns.clear();links=mat.node_tree.links
 geo=ns.new('ShaderNodeNewGeometry');mix=ns.new('ShaderNodeMixRGB');links.new(geo.outputs['Backfacing'],mix.inputs[0])
 for face in range(2):
  tex=ns.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(ATLAS/f'wing-{side*2+face}.png'));tex.image.pack();tex.interpolation='Closest';links.new(tex.outputs['Color'],mix.inputs[face+1])
 em=ns.new('ShaderNodeEmission');links.new(mix.outputs[0],em.inputs[0]);out=ns.new('ShaderNodeOutputMaterial');links.new(em.outputs[0],out.inputs[0]);return mat

def main():
 assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;OUT.mkdir()
 fit=json.loads(FIT.read_text());outline=np.array(fit['fixed_geometry']['wing_outline']);bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene
 s.render.engine='CYCLES';s.cycles.samples=4;s.cycles.use_denoising=False;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None';s.render.fps=25
 s.world=bpy.data.worlds.new('Neutral');s.world.use_nodes=True;s.world.node_tree.nodes['Background'].inputs[0].default_value=(.3,.3,.3,1)
 rig=bpy.data.objects.new('Butterfly07 fixed anatomy local motion',None);s.collection.objects.link(rig);rig.rotation_mode='QUATERNION';rig['source_animation_index']=14;rig['depth_inferred']=True
 bpy.ops.mesh.primitive_uv_sphere_add(segments=16,ring_count=8);body=bpy.context.object;body.name='Unknown body appearance';body.parent=rig
 body.scale=(.5,2.7,.5);body.matrix_basis=Matrix(B.tolist()).to_4x4()@body.matrix_basis
 gray=bpy.data.materials.new('Unknown neutral gray');gray.diffuse_color=(.35,.35,.35,1);body.data.materials.append(gray)
 wings=[]
 for side,sign in enumerate((-1,1)):
  verts=outline.copy();verts[:,0]*=sign;verts=verts@B.T;faces=[[0,i,i+1] for i in range(1,len(verts)-1)]
  if sign==1:faces=[list(reversed(f)) for f in faces]
  mesh=bpy.data.meshes.new(f'Fixed wing {side}');mesh.from_pydata(verts.tolist(),[],faces);mesh.update();uv=mesh.uv_layers.new(name='Observed wing coordinates')
  for poly in mesh.polygons:
   for li in poly.loop_indices:
    v=outline[mesh.loops[li].vertex_index];uv.data[li].uv=(v[0]/8,1-(v[1]+7)/13)
  ob=bpy.data.objects.new(mesh.name,mesh);s.collection.objects.link(ob);ob.parent=rig;ob.location=Vector(B@np.array([fit['fixed_geometry']['wing_hinge_offsets'][side],0,0]));ob.rotation_mode='QUATERNION';mesh.materials.append(material(side));wings.append(ob)
 for phase in range(100):
  row=fit['rows'][phase%99];p=row['parameters'];frame=phase*2+1;rotation=rot('z',p[2])@rot('y',p[1])@rot('x',p[0]);rig.rotation_quaternion=Matrix((B@rotation@B.T).tolist()).to_quaternion();rig.keyframe_insert('rotation_quaternion',frame=frame)
  rig.location=Vector(B@np.array([p[5],p[6],0.]));rig.keyframe_insert('location',frame=frame)
  for side,sign in enumerate((-1,1)):
   wings[side].rotation_quaternion=Matrix((B@rot('y',-sign*p[3+side])@B.T).tolist()).to_quaternion();wings[side].keyframe_insert('rotation_quaternion',frame=frame)
 s.frame_start=1;s.frame_end=198;s.frame_set(5)
 camera=bpy.data.cameras.new('Original art direction first');camera.type='ORTHO';camera.ortho_scale=24;cam=bpy.data.objects.new(camera.name,camera);s.collection.objects.link(cam);s.camera=cam
 for name,offset,energy in [('key',(-25,-35,50),22000),('fill',(35,10,20),9000)]:
  data=bpy.data.lights.new(name,'AREA');data.energy=energy;data.size=25;ob=bpy.data.objects.new(name,data);s.collection.objects.link(ob);ob.location=offset;ob.rotation_euler=(-Vector(offset)).to_track_quat('-Z','Y').to_euler()
 bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));s=bpy.context.scene;cam=s.camera
 solid=bpy.data.materials.new('Solid inspection');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.48,.48,.48,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.8
 s.render.resolution_x=s.render.resolution_y=256;views=[]
 for mode in ('actual','solid'):
  folder=OUT/mode;folder.mkdir();s.view_layers[0].material_override=solid if mode=='solid' else None
  for i in range(8):
   angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=direction*100;cam.rotation_euler=(-cam.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();s.render.filepath=str(folder/f'view-{i}.png');bpy.ops.render.render(write_still=True)
   if mode=='actual':views.append(dict(index=i,camera_matrix_world=[list(r) for r in cam.matrix_world],ortho_scale=24))
  sheet=Image.new('RGB',(1024,512),'#252525')
  for i in range(8):im=Image.open(folder/f'view-{i}.png').convert('RGBA');sheet.paste(im,(i%4*256,i//4*256),im)
  sheet.save(folder/'sheet.png')
 s.view_layers[0].material_override=None;s.render.resolution_x=s.render.resolution_y=128;cam.matrix_world=Matrix(views[0]['camera_matrix_world']);folder=OUT/'native-motion';folder.mkdir();frames=[];sheet=Image.new('RGB',(1408,1152),'#252525');draw=ImageDraw.Draw(sheet)
 for phase in range(99):
  s.frame_set(phase*2+1);s.render.filepath=str(folder/f'{phase:03}.png');bpy.ops.render.render(write_still=True);im=Image.open(s.render.filepath).convert('RGBA');base=Image.new('RGBA',(128,128),'#252525');base.alpha_composite(im);sheet.paste(base.convert('RGB'),(phase%11*128,phase//11*128));draw.text((phase%11*128,phase//11*128),str(phase),fill='white');frames.append(base.convert('RGB'))
 sheet.save(OUT/'native-motion-sheet.png');frames[0].save(OUT/'native-motion.gif',save_all=True,append_images=frames[1:],duration=80,loop=0)
 report=dict(status='PRIVATE_MODEL_REVIEW_REQUIRED',model_sha256=sha(OUT/'model.blend'),fit_sha256=sha(FIT),atlas_report_sha256=sha(ATLAS/'report.json'),source_direction_first=True,cycle_ticks=198,rendered_phases=99,views=views,limits=['Local articulated motion only; flight path integration remains in pinned fit and inferred depth corridor.','99 key poses exact; intermediate Blender quaternion interpolation is not certified collision motion.','Observed wing texture aggregate is a proposal with per-frame shading discrepancy; gray areas unknown.','Body appearance unassigned; no generated texture and no approval claim.','No canonical or shared runtime changes.'])
 (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('SAVED_MODEL_REVIEW_COMPLETE')
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
