"""Import the actual animated GLB and review original-camera flight beside current trees."""
import sys,json,math,hashlib,shutil,argparse,importlib.util
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
S=math.sin(math.radians(35));C=math.cos(math.radians(35));sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--model',type=Path,required=True);parser.add_argument('--fit',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);MODEL=args.model;OUT=args.output;fit=json.loads(args.fit.read_text());label=f"Butterfly{fit['animation_index']-7:02}"
 memory={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())};assert memory['MemAvailable']>=6*1024**3
 assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;OUT.mkdir();bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(MODEL));root=bpy.data.objects[label+' full inferred flight path'];owned=list(bpy.context.scene.objects);culling=[]
 def render_culled():
  changed=[];removed=0
  for ob in owned:
   if ob.type!='MESH' or not ob.name.startswith('Rigid wing hinge'):continue
   original=ob.data;copy=original.copy();bm=bmesh.new();bm.from_mesh(copy);normal_matrix=ob.matrix_world.to_3x3().inverted().transposed();back=[face for face in bm.faces if (normal_matrix@face.normal).dot(s.camera.location-ob.matrix_world@face.calc_center_median())<=0];removed+=len(back);bmesh.ops.delete(bm,geom=back,context='FACES');bm.to_mesh(copy);bm.free();ob.data=copy;changed.append((ob,original,copy))
  try:bpy.ops.render.render(write_still=True)
  finally:
   for ob,original,copy in changed:ob.data=original;bpy.data.meshes.remove(copy)
  culling.append(dict(frame=s.frame_current,removed_back_faces=removed))
 s.render.engine='CYCLES';s.cycles.samples=4;s.cycles.use_denoising=False;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None';s.world=bpy.data.worlds.new('Neutral');s.world.use_nodes=True;s.world.node_tree.nodes['Background'].inputs[0].default_value=(.3,.3,.3,1)
 camera=bpy.data.cameras.new('Original camera first');camera.type='ORTHO';camera.ortho_scale=24;cam=bpy.data.objects.new(camera.name,camera);s.collection.objects.link(cam);s.camera=cam;s.render.resolution_x=s.render.resolution_y=256;views=[];isolated=OUT/'isolated';isolated.mkdir();s.frame_set(0)
 for i in range(8):
  angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*C,math.sin(angle)*C,S));center=root.matrix_world.translation;cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();s.render.filepath=str(isolated/f'{i}.png');render_culled();views.append(dict(index=i,camera_matrix_world=[list(r) for r in cam.matrix_world],ortho_scale=24))
 sheet=Image.new('RGB',(1024,512),'#252525')
 for i in range(8):im=Image.open(isolated/f'{i}.png').convert('RGBA');sheet.paste(im,(i%4*256,i//4*256),im)
 sheet.save(OUT/'isolated-eight.png');positions=[];pose_errors=[]
 helper=args.fit.parent/'geometry.py';spec=importlib.util.spec_from_file_location('own_geometry',helper);geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo)
 basis=np.array([[1,0,0],[0,-S,-C],[0,-C,S]])
 from scipy.spatial import cKDTree
 folder=OUT/'anatomy';folder.mkdir();frames=[];s.render.resolution_x=s.render.resolution_y=256
 for phase,row in enumerate(fit['rows']):
  s.frame_set(2*phase);bpy.context.view_layer.update();anchor=np.asarray(row['fixed_path_anchor_zup']);body,wings=geo.geometry(row['parameters']);errors=[]
  for side,wing in enumerate(wings):
   ob=bpy.data.objects[f'Rigid wing hinge {side}'];actual=np.array([ob.matrix_world@v.co for v in ob.data.vertices]);wanted=anchor+(wing+np.array([row['parameters'][5],row['parameters'][6],0]))@basis.T
   error=float(cKDTree(actual).query(wanted)[0].max());assert error<.001,(phase,side,error);errors.append(error)
  pose_errors.append(dict(phase=phase,maximum_vertex_error=max(errors)));positions.append(list(root.matrix_world.translation))
  center=Vector(anchor);direction=Vector((0,-C,S));cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();camera.ortho_scale=24*fit['fixed_uniform_scale'];s.render.filepath=str(folder/f'{phase:03}.png');render_culled()
  im=Image.open(s.render.filepath).convert('RGBA');base=Image.new('RGBA',im.size,'#252525');base.alpha_composite(im);draw=ImageDraw.Draw(base);draw.text((4,4),f'Own source {phase}; following art camera',fill='white');frames.append(base.convert('RGB'))
  (OUT/'progress.json').write_text(json.dumps(dict(model_sha256=sha(MODEL),fit_sha256=sha(args.fit),phase_rows=pose_errors),indent=2)+'\n')
 frames[0].save(OUT/'anatomy-motion.gif',save_all=True,append_images=frames[1:],duration=80,loop=0)
 phase_sheet=Image.new('RGB',(11*128,9*128),'#252525')
 for i,frame in enumerate(frames):phase_sheet.paste(frame.resize((128,128)),(i%11*128,i//11*128))
 phase_sheet.save(OUT/'all-source-poses.png')
 report=dict(status='ACTUAL_GLB_IMPORT_REVIEW_PENDING',source_index=fit['animation_index'],model_sha256=sha(MODEL),fit_sha256=sha(args.fit),geometry_helper_sha256=sha(helper),views=views,root_positions=positions,pose_errors=pose_errors,review_renderer=dict(policy='Only the glTF one-sided wing backfaces are excluded from temporary Cycles review meshes; original GLB unchanged.',culling=culling),limits=['Isolated anatomy review only; follow-camera GIF does not show map-relative flight or neighbors.','All99 native-time imported wing poses checked; dense continuous motion uses separate core-track proof.','Geometry and own observed body/wing appearance remain unapproved; source pixel mixtures disclosed.','No inference that static receiver contact proof covers moving neighboring states.'])
 (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('IMPORTED_ANATOMY_REVIEW_COMPLETE')
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
