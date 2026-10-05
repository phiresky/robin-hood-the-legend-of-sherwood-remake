"""Rebind preserved state artwork to the exact approved terrain and bank surfaces."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY
from render_slots import acquire,release
DEST=OUT/'restart2-state/receiver-rebind-v2'
MODELS=[('ground',OUT/'restart2-ground-completion/approved-fill-retry-v2/bake-v1/model.blend','16c638be71eeb76e86439a0fdb14bac1e7bb9562afe20d175b58d0df96fb4ec2'),('bank',OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend','69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641')]
def main():
 if DEST.exists():raise FileExistsError(DEST)
 acquire()
 try:
  DEST.mkdir();old=OUT/'state-ground-receivers-v2/receiver-transitions.json';ledger=json.loads(old.read_text());vertices=[];faces=[];labels=[];objects=[]
  for label,path,digest in MODELS:
   if sha(path)!=digest:raise ValueError('Receiver changed '+str(path))
   bpy.ops.wm.open_mainfile(filepath=str(path))
   if label=='bank':
    workspace=json.loads((path.parent/'workspace.json').read_text());bpy.context.window.scene=bpy.data.scenes[workspace['scene_name']];selected=[o for o in bpy.data.collections[workspace['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==workspace['asset_id']]
    if len(selected)!=5:raise ValueError('Expected exactly five owned bank meshes')
   else:selected=[bpy.data.objects['Croisement02 Terrain']]
   bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get()
   for obj in selected:
    evaluated=obj.evaluated_get(deps);mesh=evaluated.to_mesh();mesh.calc_loop_triangles();offset=len(vertices);matrix=evaluated.matrix_world.copy()
    vertices.extend([matrix@v.co for v in mesh.vertices]);faces.extend([tuple(offset+i for i in t.vertices) for t in mesh.loop_triangles]);labels.extend([len(objects)]*len(mesh.loop_triangles));objects.append({'receiver':label,'name':obj.name,'matrix_world':[list(r) for r in matrix],'vertices':len(mesh.vertices),'triangles':len(mesh.loop_triangles)});evaluated.to_mesh_clear()
  tree=BVHTree.FromPolygons(vertices,faces,all_triangles=True);cache={};records=[]
  for row in ledger['frames']:
   source=Path(row['source'])
   if sha(source)!=row['source_sha256']:raise ValueError('Source changed')
   alpha=np.asarray(Image.open(source).convert('RGBA'))[:,:,3]>0;x,y,w,h=row['bbox'];domains={};hits=[]
   for py,px in np.argwhere(alpha):
    key=(int(x+px),int(y+py))
    if key not in cache:
     hit,normal,face,distance=tree.ray_cast(Vector((key[0]+.5,-(key[1]+.5)/SIN,0))+RAY*5000,-RAY)
     cache[key]=None if hit is None else (labels[face],list(hit),face)
    found=cache[key]
    if found is None:raise ValueError('Unresolved receiver '+str(key))
    owner,point,face=found;domains.setdefault(owner,np.zeros_like(alpha))[py,px]=True;hits.append([int(px),int(py),owner,*point,face])
   folder=DEST/row['patch']/f"{row['state']}-{row['frame']:03}";folder.mkdir(parents=True);receivers=[]
   for owner,domain in domains.items():
    path=folder/f'receiver-{owner:02}.png';Image.fromarray(domain.astype('uint8')*255).save(path);receivers.append({'object':owner,'pixels':int(domain.sum()),'domain':str(path),'sha256':sha(path)})
   if sum(r['pixels'] for r in receivers)!=int(alpha.sum()):raise ValueError('Lost source ownership')
   record={k:v for k,v in row.items() if k!='receivers'};record['receivers']=receivers;records.append(record)
  np.savez_compressed(DEST/'ray-bindings.npz',pixels=np.array(list(cache)),hits=np.array([v[1] for v in cache.values()]),objects=np.array([v[0] for v in cache.values()]),triangles=np.array([v[2] for v in cache.values()]))
  write_json(DEST/'report.json',{'status':'PASS scoped receiver rebinding; no applied material or full-scene occlusion claim','prior_ledger_sha256':sha(old),'models':[{'receiver':label,'path':str(path),'sha256':digest}for label,path,digest in MODELS],'objects':objects,'frames':records,'unique_source_rays':len(cache),'ray_bindings_sha256':sha(DEST/'ray-bindings.npz'),'limits':['Only log/rock reserved background domains in prior ledger.','Visible props and foliage are absent from this receiver-only test.','No material applied; native alpha/color compositing and script events remain independent.']})
  print('PASS',len(records),'frames',len(cache),'source rays',flush=True)
 finally:release()
if __name__=='__main__':main()
