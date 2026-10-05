"""Double precision source-ray verification of the bounded root correction."""
import sys,json
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'restart3-tree06-root';probe=json.loads((base/'probe.json').read_text())
 region=next(r for r in json.loads((OUT/'restart3-northern-source-audit/report.json').read_text())['regions'] if r['region']==8)
 rows=[r for r in region['pixels'] if r['classification']=='wood_domain_residual']
 q=np.array([r['pixel'] for r in rows],float)+.5
 outputs=[]
 for model in [Path(probe['model']),base/'depth-v1/model.blend']:
  bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
  depth=np.full(len(q),-np.inf);which=np.full(len(q),-1);names=[]
  for obj in [o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-06' and 'wood' in o.name]:
   index=len(names);names.append(obj.name);mesh=obj.data;mesh.calc_loop_triangles()
   w=np.array([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);xy=np.column_stack((w[:,0],-SIN*w[:,1]-COS*w[:,2]))
   for tri in mesh.loop_triangles:
    ids=list(tri.vertices);a,b,c=xy[ids];basis=np.column_stack((b-a,c-a))
    if abs(np.linalg.det(basis))<1e-12:continue
    insidebox=np.all(q>=xy[ids].min(0)-1e-7,axis=1)&np.all(q<=xy[ids].max(0)+1e-7,axis=1)
    ii=np.flatnonzero(insidebox)
    if not len(ii):continue
    bc=(q[ii]-a)@np.linalg.inv(basis).T
    keep=(bc[:,0]>=-1e-7)&(bc[:,1]>=-1e-7)&(bc.sum(1)<=1+1e-7)
    ii=ii[keep];bc=bc[keep]
    if not len(ii):continue
    bary=np.column_stack((1-bc.sum(1),bc));d=(bary@w[ids])@np.array(RAY)
    take=d>depth[ii];depth[ii[take]]=d[take];which[ii[take]]=index
  result=[]
  for i,row in enumerate(rows):
   result.append(dict(pixel=row['pixel'],object=names[which[i]] if which[i]>=0 else None,ray_clearance=float(depth[i]-np.array(row['current_world_hit'])@np.array(RAY)) if np.isfinite(depth[i]) else None))
  outputs.append(dict(model=str(model),model_sha256=sha(model),covered=sum(r['object'] is not None for r in result),clear=sum(r['ray_clearance'] is not None and r['ray_clearance']>0 for r in result),pixels=result))
 write_json(base/'exact-coverage.json',dict(method='Double precision projected triangle barycentric coverage; solid wood only',models=outputs));print([(r['covered'],r['clear'])for r in outputs],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
