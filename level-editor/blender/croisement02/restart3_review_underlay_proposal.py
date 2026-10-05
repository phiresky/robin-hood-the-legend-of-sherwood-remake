"""Compare a bounded floor appearance proposal in unchanged endpoint context."""
import json,sys
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
from tree_geometry import RAY
from restart3_fence_receiver import atlas
from restore_ground75_source import geometry
from restart2_review_ground_continuation_converged import import_glb
BASE=OUT/'restart2-state'
DEST=BASE/'underlay-context-proposal-v1'
FAMILIES={
 'log-trap':['croisement02-log-trap-applied'],
 'rock-trap':['croisement02-rock-trap-applied'],
 'south-cart':['croisement02-south-cart-terminal-wreck-body','croisement02-south-cart-terminal-cask','croisement02-south-cart-terminal-loose-wood'],
 'north-cart':['croisement02-north-cart-terminal-physical'],
}
NEIGHBORS={'log-trap':[3,4,5,6],'rock-trap':[3,4,5,6],'south-cart':[43,45,46],'north-cart':[18,19]}
def main():
 global DEST,FAMILIES
 revealed='--reveal-rock' in sys.argv
 if revealed:
  DEST=BASE/'underlay-rock-revealed-diagnostic-v1'
  FAMILIES={'rock-trap':['croisement02-rock-trap-applied']}
 selection_path=Path(sys.argv[sys.argv.index('--')+1]);selection=json.loads(selection_path.read_text())
 proposal_dir=BASE/'underlay-input-review-v1';proposal=json.loads((proposal_dir/'proposal.json').read_text())
 for name,digest in proposal['images'].items():assert sha(proposal_dir/name)==digest
 ground=Path(proposal['receiver_model']);assert sha(ground)==proposal['receiver_sha256']
 if DEST.exists():raise FileExistsError(DEST)
 wanted={f'croisement02-tree-{i:02}' for values in NEIGHBORS.values()for i in values}|{'croisement02-shrub-62'}
 selected=[r for r in selection['records']if r['asset_id']in wanted];assert {r['asset_id']for r in selected}==wanted
 acquire()
 try:
  DEST.mkdir();neighbors=[]
  for row in selected:
   path=Path(row['model']);assert sha(path)==row['model_sha256'];worker=Path(row['worker']);workspace=json.loads((worker/'workspace.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.window.scene=bpy.data.scenes[workspace['scene_name']];bpy.context.view_layer.update();names=json.loads((worker/'modified/views.json').read_text())['object_names'];neighbors.append(dict(row,names=names,expected={n:{'matrix_world':[list(r)for r in bpy.data.objects[n].matrix_world]}for n in names}))
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.resolution_x=scene.render.resolution_y=640;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
  sun=bpy.data.objects.new('Sun',bpy.data.lights.new('Sun','SUN'));scene.collection.objects.link(sun);sun.data.energy=2;sun.rotation_euler=(-Vector((-.45,-.55,.70))).to_track_quat('-Z','Y').to_euler();cam=bpy.data.objects.new('Camera',bpy.data.cameras.new('Camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.clip_end=20000
  ledger=json.loads((BASE/'receiver-rebind-v2/report.json').read_text());imports=[]
  for model in ledger['models']:
   path=ground if model['receiver']=='ground'else Path(model['path']);assert sha(path)==(proposal['receiver_sha256']if model['receiver']=='ground'else model['sha256']);rows=[r for r in ledger['objects']if r['receiver']==model['receiver']];_,proof=append_verified(scene,path,[r['name']for r in rows],{r['name']:r for r in rows});imports+=proof
  terrain=bpy.data.objects['Croisement02 Terrain'];signature=geometry(terrain);node,pixels=atlas(terrain);assert np.array_equal(pixels,np.array(Image.open(proposal_dir/'input.png')));original_image=node.image;candidate_image=bpy.data.images.load(str(proposal_dir/'proposed-appearance.png'));neighbor_objects={}
  for n in neighbors:
   objects,proof=append_verified(scene,Path(n['model']),n['names'],n['expected']);neighbor_objects[n['asset_id']]=objects;imports+=proof
  endpoints={}
  for version in [1,2]:
   directory=BASE/f'approved-physical-endpoint-exports-v{version}'
   for row in json.loads((directory/'manifest.json').read_text())['records']:
    if not any(row['id']in ids for ids in FAMILIES.values()):continue
    path=directory/row['glb'];assert sha(path)==row['glb_sha256'];objects=import_glb(path);points=np.array([o.matrix_world@v.co for o in objects if o.type=='MESH'for v in o.data.vertices]);lo=points.min(0);hi=points.max(0);assert max(abs(lo-np.min([p['bounds'][0]for p in row['parts']],0)))<.002;assert max(abs(hi-np.max([p['bounds'][1]for p in row['parts']],0)))<.002;endpoints[row['id']]={'objects':objects,'lo':lo,'hi':hi,'row':row}
  records=[]
  for family,ids in FAMILIES.items():
   for key,g in endpoints.items():
    for o in g['objects']:o.hide_render=key not in ids
   visible={f'croisement02-tree-{i:02}'for i in NEIGHBORS[family]}
   if family in ['log-trap','rock-trap']:visible.add('croisement02-shrub-62')
   if revealed:visible.clear()
   for key,objects in neighbor_objects.items():
    for o in objects:o.hide_render=key not in visible
   lo=np.min([endpoints[k]['lo']for k in ids],0);hi=np.max([endpoints[k]['hi']for k in ids],0);center=Vector((lo+hi)/2);cam.data.ortho_scale=max(240,float(np.linalg.norm(hi-lo))*1.35)
   for view,direction in [('native',RAY),('oblique',Vector((1,-1,.8)).normalized())]:
    cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler()
    for state,img in [('before',original_image),('proposal',candidate_image)]:
     node.image=img;bpy.context.view_layer.update();name=f'{family}-{view}-{state}.png';scene.render.filepath=str(DEST/name);bpy.ops.render.render(write_still=True);records.append({'family':family,'view':view,'appearance':state,'file':name,'sha256':sha(DEST/name),'direction':list(direction)})
  assert geometry(terrain)==signature
  sheet=Image.new('RGB',(1280,1280),'#333333')
  for y,family in enumerate(FAMILIES):
   for x,(view,state)in enumerate([('native','before'),('native','proposal'),('oblique','before'),('oblique','proposal')]):
    im=Image.open(DEST/f'{family}-{view}-{state}.png').convert('RGBA').resize((320,320));bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(x*320,y*320))
  sheet.save(DEST/'comparison.png');write_json(DEST/'manifest.json',{'status':'Private proposed floor appearance visualization; no saved model or approval','proposal_sha256':sha(proposal_dir/'proposal.json'),'selection_sha256':sha(selection_path),'neighbors':neighbors,'imports':imports,'endpoints':[v['row']for v in endpoints.values()],'records':records,'geometry_unchanged':True,'revealed_diagnostic_all_vegetation_hidden':revealed,'limits':['Only selected local context; not whole-map occlusion proof.','Dynamic source receiver patches are intentionally not shown: this compares the underlying inferred floor.','Other unassigned gray areas remain unchanged.','No API, canonical model, catalog or publication change.']})
 finally:release()
if __name__=='__main__':main()
