"""Read-only exact new shed member intersections with frozen neighboring wood."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 worker=OUT/'restart4-source-gaps/shed-east-v1';e=json.loads((worker/'wide-contact-v1/evidence.json').read_text());bpy.ops.wm.read_factory_settings(use_empty=True);objects=[]
 for rec in e['inputs']:
  if rec['role']!='candidate'and rec['asset']!='croisement02-tree-47':continue
  assert sha(Path(rec['model']))==rec['model_sha256']
  with bpy.data.libraries.load(rec['model'],link=False)as(src,dst):dst.objects=[r['name']for r in rec['objects']]
  for o in dst.objects:
   bpy.context.scene.collection.objects.link(o);parent=o.parent
   while parent:
    if not parent.users_collection:bpy.context.scene.collection.objects.link(parent)
    parent=parent.parent
  bpy.context.view_layer.update()
  for o,r in zip(dst.objects,rec['objects']):assert np.max(np.abs(np.array(o.matrix_world)-np.array(r['matrix'])))<1e-5
  objects+=dst.objects
 new=next(o for o in objects if o.name.startswith('Inferred complete east roof'));wood=[o for o in objects if o.get('asset_group')=='croisement02-tree-47'and o.get('projection_component')!='crown'and 'crown'not in o.name.lower()]
 def mesh(o):
  verts=[o.matrix_world@v.co for v in o.data.vertices];return verts,BVHTree.FromPolygons(verts,[list(p.vertices)for p in o.data.polygons])
 nv,nt=mesh(new);rows=[]
 for o in wood:
  ov,ot=mesh(o);hits=[]
  for edge in o.data.edges:
   a,b=[ov[i]for i in edge.vertices];d=b-a
   if d.length<1e-7:continue
   p,n,index,dist=nt.ray_cast(a,d.normalized(),d.length)
   if p is not None and dist>1e-5 and dist<d.length-1e-5:hits.append(dict(kind='wood_edge',edge=edge.index,point=list(p)))
  for edge in new.data.edges:
   a,b=[nv[i]for i in edge.vertices];d=b-a
   if d.length<1e-7:continue
   p,n,index,dist=ot.ray_cast(a,d.normalized(),d.length)
   if p is not None and dist>1e-5 and dist<d.length-1e-5:hits.append(dict(kind='shed_edge',edge=edge.index,point=list(p)))
  rows.append(dict(object=o.name,component=o.get('projection_component'),vertices=len(ov),bounds=[np.min(ov,axis=0).tolist(),np.max(ov,axis=0).tolist()],intersections=hits))
 out=worker/'wood-intersections.json';assert not out.exists();write_json(out,dict(model_sha256=e['model_sha256'],context_evidence_sha256=sha(worker/'wide-contact-v1/evidence.json'),method='Bidirectional finite actual mesh edge versus other surface triangle ray intersections; crown excluded.',objects=rows,total_intersections=sum(len(r['intersections'])for r in rows)))
 print(out)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
