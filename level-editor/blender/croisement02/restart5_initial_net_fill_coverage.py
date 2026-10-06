"""Separate sampled real surface fill gaps from atlas padding and hidden ground backs."""
import sys,json
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
ROOT=OUT/'restart5-initial-nets'
def main(key):
 exp=ROOT/f'texture-fill-v1/profile-{key}/experiment';w=exp/'native-retained-v2';bake=exp/('bake-v3-identity' if key=='00'else 'bake-v2-identity');layers=json.loads((bake/'layer-0.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(w/'worker.blend'));scene=bpy.context.scene;rows=[]
 for rec in layers['objects']:
  o=scene.objects[rec['object']];path=Path(rec['texel_provenance']['path']);assert sha(path)==rec['texel_provenance']['sha256'];a=np.load(path)['ownership'];uvname=next(n.inputs['Vector'].links[0].from_node.uv_map for m in o.data.materials if m and m.get('source_ownership_bake')for n in m.node_tree.nodes if n.type=='TEX_IMAGE'and n.image);uv=o.data.uv_layers[uvname];counts={};gaps=[];o.data.calc_loop_triangles()
  for t in o.data.loop_triangles:
   q=sum((np.asarray(uv.data[i].uv)for i in t.loops))/3;x=max(0,min(a.shape[1]-1,int(q[0]*a.shape[1])));y=max(0,min(a.shape[0]-1,int(q[1]*a.shape[0])));code=int(a[y,x]);counts[str(code)]=counts.get(str(code),0)+1
   if code==0:gaps.append(dict(face=t.polygon_index,normal_z=float(o.data.polygons[t.polygon_index].normal.z),centroid=[float(v)for v in sum((np.array(o.matrix_world@o.data.vertices[i].co)for i in t.vertices))/3]))
  rows.append(dict(object=o.name,triangle_center_provenance_counts=counts,unfilled_samples=gaps,ground_downward_unfilled=sum(g['normal_z']<-.5 for g in gaps)if o.name=='Ground camouflage net'else 0))
 write_json(w/'fill-coverage.json',dict(model_sha256=sha(w/'worker.blend'),scope='Finite triangle-center atlas provenance diagnostic; not exhaustive surface coverage or padding count',objects=rows));print([(r['object'],r['triangle_center_provenance_counts'],r['ground_downward_unfilled'])for r in rows])
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
