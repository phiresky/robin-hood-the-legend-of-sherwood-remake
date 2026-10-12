"""Bound visible unknown edge samples to nearby generated bark on connected wood faces."""
import json,shutil,sys
from pathlib import Path
from array import array
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from review_evidence import sha
from refinement_workspace import _geometry
from render_multiview_asset import render
from refinement_review import _tile
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';src=W/'baked-v5-bounded-gaps';out=W/'baked-v8-connected-gaps'
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');mesh=obj.data;before=_geometry(obj)
 provenance={}
 for label,folder in [('exterior',W/'baked-v1'),('exterior / visible-gap-v1',src)]:
  provenance[label]=np.load(json.loads((folder/'layer-0.json').read_text())['objects'][0]['texel_provenance']['path'])['ownership'].copy()
 slots={};images={}
 for slot,mat in enumerate(mesh.materials):
  if not mat or not mat.get('source_ownership_bake'):continue
  texture=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');im=texture.image;pixels=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(pixels);pixels=pixels.reshape(im.size[1],im.size[0],4)
  images[im.name]={'image':im,'pixels':pixels,'before':pixels.copy(),'ownership':provenance[mat['source_ownership_label']]};slots[slot]=(im.name,mesh.uv_layers[next(n.uv_map for n in mat.node_tree.nodes if n.type=='UVMAP')]);texture.interpolation='Closest'
 protected={(r['image'],*r['texel']) for r in json.loads((src/'native-texel-samples.json').read_text())}
 world=np.array([obj.matrix_world@v.co for v in mesh.vertices]);faces=np.array([p.vertices[:] for p in mesh.polygons]);centers=world[faces].mean(1);normals=np.array([p.normal[:] for p in mesh.polygons]);links=[[] for _ in world]
 for i,vs in enumerate(faces):
  for v in vs:links[v].append(i)
 cache={}
 def donors(fid):
  if fid in cache:return cache[fid]
  face=mesh.polygons[fid]
  if face.material_index not in slots:return []
  result=[]
  for slot in dict.fromkeys([face.material_index]+list(slots)):
   name,uv=slots[slot];d=images[name];size=np.array(d['image'].size);coords=np.array([uv.data[i].uv[:] for i in face.loop_indices])*size
   if np.max(np.ptp(coords,axis=0))<1e-5:continue
   lo=np.maximum(0,np.floor(coords.min(0)).astype(int)-1);hi=np.minimum(size,np.ceil(coords.max(0)).astype(int)+1)
   points=np.argwhere(d['ownership'][lo[1]:hi[1],lo[0]:hi[0]]==2)
   if len(points):points+=lo[::-1];center=coords.mean(0);points=sorted(points,key=lambda p:float(np.sum((p[::-1]-center)**2)))
   result.extend((name,int(p[1]),int(p[0])) for p in points)
  cache[fid]=result;return result
 audit=json.loads((src/'visible-fill-audit/report.json').read_text());assert audit['model_sha256']==sha(src/'worker.blend');repairs={};failed=[]
 for view in audit['views']:
  for _,_,fid,x,y in view['unfilled']:
   name,_=slots[mesh.polygons[fid].material_index];key=(name,x,y)
   if key in repairs:continue
   assert images[name]['ownership'][y,x]==0 and (name,x,images[name]['image'].size[1]-1-y) not in protected
   seen={fid};front={fid};chosen=None
   for ring in range(9):
    viable=sorted(front,key=lambda f:float(np.linalg.norm(centers[f]-centers[fid])))
    for other in viable:
     distance=float(np.linalg.norm(centers[other]-centers[fid]))
     if distance>8 or normals[other]@normals[fid]<.25:continue
     candidates=donors(other)
     if candidates:chosen=(other,distance,ring,candidates[0]);break
    if chosen:break
    front={j for f in front for v in faces[f] for j in links[v]}-seen;seen|=front
   if not chosen:failed.append([fid,x,y]);continue
   other,distance,ring,(donor,dx,dy)=chosen;images[name]['pixels'][y,x,:3]=images[donor]['before'][dy,dx,:3]
   repairs[key]={'face':fid,'image':name,'texel':[x,y],'donor_face':other,'donor_image':donor,'donor_texel':[dx,dy],'face_center_distance_world':distance,'adjacency_rings':ring}
 assert len(repairs)<=1000;assert not out.exists();out.mkdir()
 if failed:
  (out/'unresolved.json').write_text(json.dumps({'safe_repairs':len(repairs),'unresolved':failed},indent=2)+'\n');print('HOLD',len(repairs),len(failed),flush=True)
 else:
  for name,d in images.items():
   assert np.array_equal(d['pixels'][d['ownership']!=0],d['before'][d['ownership']!=0]);assert np.array_equal(d['pixels'][:,:,3],d['before'][:,:,3]);d['image'].pixels.foreach_set(d['pixels'].ravel());d['image'].update();d['image'].pack()
  assert before==_geometry(obj);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
  record={'status':'PASS_BOUNDED_CONNECTED_GENERATED_FILL','model_sha256':sha(out/'worker.blend'),'parent_model_sha256':sha(src/'worker.blend'),'repairs':list(repairs.values()),'geometry_exact':True,'protected_source_existing_generated_and_alpha_exact':True,'bounds':{'maximum_world_distance':8,'maximum_adjacency_rings':8,'minimum_normal_dot':.25,'maximum_target_texels':1000},'scope':'Only class-zero atlas samples visibly hit in exact eight review cameras; donor is generated bark on the same connected wood surface. No source texels changed.'};(out/'gap-repair.json').write_text(json.dumps(record,indent=2)+'\n')
  for k,(_,d) in enumerate(images.items()):
   ownership=d['ownership'].copy()
   for name,x,y in repairs:
    if name==d['image'].name:ownership[y,x]=3
   np.savez_compressed(out/f'ownership-{k}.npz',ownership=ownership)
  render(W/'experiment/baked-v5-bounded-sampling-views.json',out/'actual',width=384);buffers=[]
  for i in range(8):
   im=bpy.data.images.load(str(out/'actual'/f'view-{i}-textured.png'),check_existing=False);v=array('f',[0])*len(im.pixels);im.pixels.foreach_get(v);buffers.append(v);bpy.data.images.remove(im)
  _tile(buffers,384,384,out/'actual/textured.png');print('PASS',len(repairs),flush=True)
finally:release()
