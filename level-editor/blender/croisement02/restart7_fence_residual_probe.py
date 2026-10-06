"""Read-only source-ray probe of small wattle and masonry contour residuals."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
D=OUT/'restart7-fence-residual';INVENTORY=OUT/'restart6-source-coverage/remaining-inventory-v1/report.json'
SPECS=[(98,'croisement02-south-field-wattle-fence',OUT/'restart4-initial-fence-approved/assets/croisement02-south-field-wattle-fence/model.blend'),(101,'croisement02-east-stone-wall-and-gate',OUT/'texture-fill-round-1/croisement02-east-stone-wall-and-gate/experiment/bake-v1/worker.blend')]
def main():
 inv=json.loads(INVENTORY.read_text());records=[]
 for mask,asset,path in SPECS:
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset];trees=[];inventory=[]
  for o in objects:
   o.data.calc_loop_triangles();v=np.array([tuple(o.matrix_world@p.co)for p in o.data.vertices]);faces=[tuple(t.vertices)for t in o.data.loop_triangles];tree=BVHTree.FromPolygons(v.tolist(),faces,all_triangles=True);xy=np.column_stack([v[:,0],-SIN*v[:,1]-COS*v[:,2]]);trees.append((o,tree,v,xy));inventory.append(dict(name=o.name,source_node=o.get('source_node'),vertices=len(v),faces=len(faces),bounds=[v.min(0).tolist(),v.max(0).tolist()]))
  points=next(r['coordinates']for r in inv['rows']if r['mask']==mask);points=[p for p in points if mask!=98 or p[0]<1000];samples=[]
  for x,y in points:
   near=[];hits=[];neighbors=[]
   for o,t,v,xy in trees:
    dd=np.linalg.norm(xy-[x+.5,y+.5],axis=1);ii=int(dd.argmin());near.append(dict(object=o.name,index=ii,distance=float(dd[ii]),source_xy=xy[ii].tolist(),world=v[ii].tolist()))
    for dx,dy in [(0,0),(0,1),(0,-1),(1,0),(-1,0)]:
     origin=Vector((x+.5+dx,-(y+.5+dy)/SIN,0))+RAY*5000;h,n,f,d=t.ray_cast(origin,-RAY,10000)
     if h is not None:
      row=dict(object=o.name,source_node=o.get('source_node'),hit=list(h),distance=d,triangle=f,polygon=o.data.loop_triangles[f].polygon_index,offset=[dx,dy]);(hits if dx==dy==0 else neighbors).append(row)
   samples.append(dict(pixel=[x,y],first_hit=min(hits,key=lambda r:r['distance'])if hits else None,nearest_vertices=sorted(near,key=lambda r:r['distance'])[:3],nearby_hits=sorted(neighbors,key=lambda r:r['distance'])[:8]))
  records.append(dict(mask=mask,asset_id=asset,model=str(path),model_sha256=sha(path),objects=inventory,samples=samples))
 write_json(D/'probe-v1.json',dict(status='Read-only geometry diagnosis; solid rays do not establish alpha/material ownership',source_inventory_sha256=sha(INVENTORY),records=records));print([(r['mask'],len(r['objects']),sum(s['first_hit']is not None for s in r['samples']))for r in records])
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
