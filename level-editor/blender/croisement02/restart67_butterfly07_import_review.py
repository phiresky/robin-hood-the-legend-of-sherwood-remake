"""Import the actual animated GLB and review original-camera flight beside current trees."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
BASE=ROOT/'level-editor/work/active/croisement02';MODEL=BASE/'restart67-butterfly07-fullpath-export-v1/butterfly07-fullpath.glb';OUT=BASE/'restart67-butterfly07-import-review-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;OUT.mkdir();bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(MODEL));root=bpy.data.objects['Butterfly07 full inferred flight path'];owned=list(bpy.context.scene.objects);culling=[]
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
 camera=bpy.data.cameras.new('Original camera first');camera.type='ORTHO';camera.ortho_scale=24;cam=bpy.data.objects.new(camera.name,camera);s.collection.objects.link(cam);s.camera=cam;s.render.resolution_x=s.render.resolution_y=256;views=[];isolated=OUT/'isolated';isolated.mkdir();s.frame_set(5)
 for i in range(8):
  angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*C,math.sin(angle)*C,S));center=root.matrix_world.translation;cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();s.render.filepath=str(isolated/f'{i}.png');render_culled();views.append(dict(index=i,camera_matrix_world=[list(r) for r in cam.matrix_world],ortho_scale=24))
 sheet=Image.new('RGB',(1024,512),'#252525')
 for i in range(8):im=Image.open(isolated/f'{i}.png').convert('RGBA');sheet.paste(im,(i%4*256,i//4*256),im)
 sheet.save(OUT/'isolated-eight.png');positions=[]
 for phase in range(100):s.frame_set(phase*2+1);positions.append(list(root.matrix_world.translation))
 # Import actual map references with their current placement; keep neighbor provenance separate.
 mappath=ROOT/'level-editor/library/scenes/croisement02.rhlos-map.json';mapdoc=json.loads(mappath.read_text());sources={r['id']:r for r in mapdoc['assetSources']};pins=[]
 for placement in mapdoc['placements']:
  if placement['id'] not in ('croisement02-tree-01','croisement02-tree-02'):continue
  source=sources[placement['assets'][0]];path=ROOT/'level-editor/library'/source['model'];before=set(s.objects);bpy.ops.import_scene.gltf(filepath=str(path));added=set(s.objects)-before;t=placement['transform'];assert t['rot_deg']==0
  for ob in added:
   if ob.parent not in added:ob.location+=Vector((t['dx'],-t['dy']/S,t['dz']))
  pins.append(dict(path=str(path),sha256=sha(path),placement=placement))
 s.frame_start=1;s.frame_end=198;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'imported-context.blend'),compress=True)
 center=Vector(np.mean(positions,axis=0));direction=Vector((0,-C,S));cam.location=center+direction*400;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();camera.ortho_scale=200;s.render.resolution_x=640;s.render.resolution_y=480;folder=OUT/'flight';folder.mkdir();frames=[]
 for phase in range(99):
  s.frame_set(2*phase+1);s.render.filepath=str(folder/f'{phase:03}.png');render_culled();im=Image.open(s.render.filepath).convert('RGBA');base=Image.new('RGBA',im.size,'#252525');base.alpha_composite(im);draw=ImageDraw.Draw(base);draw.text((8,8),f'Phase{phase} /198-tick cycle; original camera; trees01+02 only',fill='white');frames.append(base.convert('RGB'))
 frames[0].save(OUT/'flight.gif',save_all=True,append_images=frames[1:],duration=80,loop=0)
 chosen=[0,8,21,27,32,34,68,88];sheet=Image.new('RGB',(1280,960),'#252525')
 for i,phase in enumerate(chosen):sheet.paste(frames[phase].resize((320,240)),(i%4*320,i//4*240))
 sheet.save(OUT/'flight-context-sheet.png');report=dict(status='ACTUAL_GLB_IMPORT_REVIEW_PENDING',model_sha256=sha(MODEL),imported_blend_sha256=sha(OUT/'imported-context.blend'),map_sha256=sha(mappath),neighbors=pins,views=views,root_positions=positions,review_renderer=dict(policy='Only the glTF one-sided wing backfaces are excluded from temporary Cycles review meshes; original GLB and saved imported blend retain both primitives.',culling=culling),limits=['Two actual trees shown as partial context; no full-map appearance claim.','Discrete phase captures do not replace independent dense core-track readback.','Geometry and exact observed body/wing appearance remain unapproved; source pixel mixtures disclosed.','No inference that static receiver proof covers moving neighboring states.']);(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('IMPORTED_FLIGHT_REVIEW_COMPLETE')
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
