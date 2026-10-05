"""Review approved trap endpoints with their exact physical receiver appearances."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from sign_context_import import append_verified
from tree_geometry import RAY,SIN,COS
BASE=OUT/'restart2-state';DEST=BASE/'trap-receiver-context-v2'
def import_glb(path):
 old=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(path));new=[o for o in bpy.data.objects if o not in old]
 for o in new:o.animation_data_clear()
 bpy.context.view_layer.update();return new
def main():
 if DEST.exists():raise FileExistsError(DEST)
 ledger=json.loads((BASE/'receiver-rebind-v2/report.json').read_text());endpoints=json.loads((BASE/'approved-physical-endpoint-exports-v1/manifest.json').read_text());patches=json.loads((BASE/'receiver-surfaces-v1/manifest.json').read_text());acquire()
 try:
  DEST.mkdir();selection_path=OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json';selection=json.loads(selection_path.read_text());neighbors=[]
  wanted={f'croisement02-tree-{i:02}'for i in [3,4,5,6]}|{'croisement02-shrub-62'}
  for selected in selection['records']:
   if selected['asset_id'] not in wanted:continue
   path=Path(selected['model']);assert sha(path)==selected['model_sha256'];worker=Path(selected['worker']);workspace=json.loads((worker/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.window.scene=bpy.data.scenes[workspace['scene_name']];bpy.context.view_layer.update();names=json.loads((worker/'modified/views.json').read_text())['object_names'];expected={name:{'matrix_world':[list(r)for r in bpy.data.objects[name].matrix_world]}for name in names};neighbors.append({'asset_id':selected['asset_id'],'model':str(path),'sha256':sha(path),'names':names,'expected':expected})
  assert len(neighbors)==5
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Trap endpoint receiver context';scene.render.engine='CYCLES';scene.cycles.samples=16;scene.render.resolution_x=scene.render.resolution_y=640;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1);sun=bpy.data.objects.new('Sun',bpy.data.lights.new('Sun','SUN'));scene.collection.objects.link(sun);sun.data.energy=2;sun.rotation_euler=(-Vector((-.45,-.55,.70))).to_track_quat('-Z','Y').to_euler();cam=bpy.data.objects.new('Native-first camera',bpy.data.cameras.new('Native-first camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.clip_end=20000;imports=[]
  for model in ledger['models']:
   path=Path(model['path']);assert sha(path)==model['sha256'];rows=[r for r in ledger['objects']if r['receiver']==model['receiver']];_,proof=append_verified(scene,path,[r['name']for r in rows],{r['name']:r for r in rows});imports.extend(proof)
  for neighbor in neighbors:
   _,proof=append_verified(scene,Path(neighbor['model']),neighbor['names'],neighbor['expected']);imports.extend(proof)
  groups=[]
  for row in endpoints['records']:
   if 'net-'in row['id']:continue
   path=BASE/'approved-physical-endpoint-exports-v1'/row['glb'];assert sha(path)==row['glb_sha256'];objects=import_glb(path);points=[o.matrix_world@v.co for o in objects if o.type=='MESH'for v in o.data.vertices];lo=np.min(np.array(points),axis=0);hi=np.max(np.array(points),axis=0);expected_lo=np.min([p['bounds'][0]for p in row['parts']],axis=0);expected_hi=np.max([p['bounds'][1]for p in row['parts']],axis=0);drift=max(np.max(abs(lo-expected_lo)),np.max(abs(hi-expected_hi)));assert drift<.002
   for o in objects:o.hide_render=True
   groups.append({'record':row,'objects':objects,'lo':lo,'hi':hi,'world_bounds_drift':float(drift)})
  surfaces={}
  for row in patches['records']:
   path=BASE/'receiver-surfaces-v1'/row['file'];assert sha(path)==row['sha256'];objects=import_glb(path)
   for o in objects:o.hide_render=True
   surfaces[row['assembly']]=(row,objects)
  bpy.context.window.scene=scene;records=[]
  for group in groups:
   row=group['record'];assembly='log-trap'if 'log-trap'in row['id']else 'rock-trap';state='initial'if row['id'].endswith('covered')else 'transition';patch,objects=surfaces[assembly];phase=0 if state=='initial'else patch['transition_frames']-1
   for g in groups:
    for obj in g['objects']:obj.hide_render=g is not group
   for family,(_,nodes)in surfaces.items():
    for obj in nodes:
     active=family==assembly and obj.get('native_state')==state and obj.get('native_frame')==phase;obj.hide_render=not active
     if obj.type=='MESH':obj.scale=(1,1,1)if active else(0,0,0)
   bpy.context.view_layer.update();center=Vector((group['lo']+group['hi'])/2);cam.data.ortho_scale=max(240,float(np.linalg.norm(group['hi']-group['lo']))*1.65);renders=[]
   for view,direction in [('native',RAY),('oblique',Vector((1,-1,.8)).normalized()),('reverse',Vector((0,1,.8)).normalized())]:
    cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();file=DEST/f"{row['id']}-{view}.png";scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);renders.append({'view':view,'file':file.name,'sha256':sha(file),'direction':list(direction)})
   records.append({'id':row['id'],'endpoint_glb_sha256':row['glb_sha256'],'endpoint_model_sha256':row['model_sha256'],'receiver_glb_sha256':patch['sha256'],'receiver_state':state,'receiver_phase':phase,'world_bounds_drift':group['world_bounds_drift'],'renders':renders})
  sheet=Image.new('RGB',(960,1280),'#333333')
  for j,row in enumerate(records):
   for i,r in enumerate(row['renders']):
    im=Image.open(DEST/r['file']).convert('RGBA').resize((320,320));bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i*320,j*320))
  sheet.save(DEST/'joint-sheet.png');write_json(DEST/'manifest.json',{'status':'Private approved endpoint+receiver context; self-review pending','selection_sha256':sha(selection_path),'static_neighbors':neighbors,'evaluated_context_imports':imports,'receiver_models':ledger['models'],'records':records,'limits':['Ground, northern bank and exact approved trees03/04/05/06 plus shrub62; not whole-map occlusion proof.','Each endpoint uses corresponding initial/retained terminal native receiver phase.','No transition rigid motion or atlas restoration beyond explicit patch pixels.']})
 finally:release()
if __name__=='__main__':main()
