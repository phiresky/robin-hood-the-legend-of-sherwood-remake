"""Probe exact accepted tree root rays before a bounded source-preserving correction."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json

def main():
 out=OUT/'restart3-tree06-root';out.mkdir(exist_ok=True)
 row=next(r for r in json.loads((OUT/'restart2-textures/batch-v3-coherent-selection-v1/selection.json').read_text())['records']if r['asset_id']=='croisement02-tree-06');model=Path(row['model']);assert sha(model)==row['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
 region=next(r for r in json.loads((OUT/'restart3-northern-source-audit/report.json').read_text())['regions']if r['region']==8);samples=[s for s in region['pixels']if s['classification']=='wood_domain_residual'];objects=[o for o in bpy.data.objects if o.type=='MESH'and o.get('asset_group')==row['asset_id']and o.get('projection_component')!='crown'and 'wood'in o.name.lower()];assert len(objects)==3
 rows=[];points=[]
 for o in objects:
  o.data.calc_loop_triangles();world=[tuple(o.matrix_world@v.co)for v in o.data.vertices];tree=BVHTree.FromPolygons(world,[tuple(t.vertices)for t in o.data.loop_triangles],all_triangles=True);bounds=np.array(world)
  rows.append(dict(name=o.name,source_node=o.get('source_node'),matrix_world=[list(r)for r in o.matrix_world],vertices=len(world),bounds_min=bounds.min(0).tolist(),bounds_max=bounds.max(0).tolist(),uvs=[u.name for u in o.data.uv_layers],materials=[dict(name=m.name,nodes=[dict(name=n.name,type=n.type,image=n.image.name if n.type=='TEX_IMAGE'and n.image else None)for n in m.node_tree.nodes])for m in o.data.materials]))
  for s in samples:
   x,y=s['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;hit,n,face,d=tree.ray_cast(origin,-RAY,10000)
   if hit is not None:
    bank=Vector(s['current_world_hit']);required=max(0,(bank-hit).dot(RAY)+.5);points.append(dict(pixel=[x,y],object=o.name,hit=list(hit),distance=d,required_native_ray_advance=required,bank=list(bank),triangle=face))
 nearest={}
 for p in points:
  key=tuple(p['pixel'])
  if key not in nearest or p['distance']<nearest[key]['distance']:nearest[key]=p
 write_json(out/'probe.json',dict(model=str(model),model_sha256=sha(model),objects=rows,sample_count=len(samples),hit_count=len(nearest),rays=list(nearest.values()),missing=[s['pixel']for s in samples if tuple(s['pixel'])not in nearest]));print(out/'probe.json')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
