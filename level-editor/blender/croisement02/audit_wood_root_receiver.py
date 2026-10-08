"""Independently intersect saved solid roots with the pinned ground receiver."""
import argparse,hashlib,json,sys
from pathlib import Path
import numpy as np
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
OUT=ROOT/'level-editor/work/croisement02-refinement'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def intersections(triangles,point,normal):
 distances=(triangles-point)@normal
 selected=triangles[(distances.min(axis=1)<0)&(distances.max(axis=1)>0)];segments=[]
 for triangle in selected:
  signed=(triangle-point)@normal;hits=[]
  for i,j in [(0,1),(1,2),(2,0)]:
   if signed[i]*signed[j]<0:hits.append(triangle[i]+(triangle[j]-triangle[i])*(signed[i]/(signed[i]-signed[j])))
  if len(hits)==2:segments.append(hits)
 return np.asarray(segments)
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--worker',type=Path,required=True);parser.add_argument('--tree',type=int,choices=[32,38],required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
 if args.output.exists():raise FileExistsError(args.output)
 model=args.worker/'model.blend';digest=sha(model);receiver=OUT/'ground-receiver-review-v5/model.blend';expected='efed22896d8c4075bb0ed346bf6a36234d28762574b5b083c546b23d09c0c0bf'
 if sha(receiver)!=expected:raise ValueError('Receiver changed')
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(receiver));obj=next(o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('source_node')=='ground');obj.data.calc_loop_triangles();rv=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);rf=[tuple(t.vertices) for t in obj.data.loop_triangles];bv=BVHTree.FromPolygons([Vector(p) for p in rv],rf,all_triangles=True);plane=rv[list(rf[0])];normal=np.cross(plane[1]-plane[0],plane[2]-plane[0]);normal/=np.linalg.norm(normal)
  if normal[2]<0:normal=-normal
  if np.max(abs((rv-plane[0])@normal))>.0003:raise ValueError('Receiver requires general nonplanar intersection')
  bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();rows=[];alltriangles=[]
  for obj in bpy.data.collections['Croisement02 Working'].all_objects:
   if obj.type!='MESH' or obj.get('asset_group')!=f'croisement02-tree-{args.tree}' or obj.get('projection_component')=='crown':continue
   obj.data.calc_loop_triangles();v=np.array([obj.matrix_world@p.co for p in obj.data.vertices]);triangles=v[np.array([tuple(t.vertices) for t in obj.data.loop_triangles])];alltriangles.append(triangles);segments=intersections(triangles,plane[0],normal);misses=0;gaps=[]
   for segment in segments:
    for point in [segment[0],segment.mean(axis=0),segment[1]]:
     hit=bv.ray_cast(Vector((point[0],point[1],100)),Vector((0,0,-1)),200)
     if hit[0] is None:misses+=1
     else:gaps.append(float(point[2]-hit[0].z))
   rows.append(dict(source_node=obj.get('source_node'),contact_segments=len(segments),contact_contour_length=float(np.linalg.norm(segments[:,1]-segments[:,0],axis=1).sum()) if len(segments) else 0.,footprint_misses=misses,maximum_receiver_plane_gap=max(map(abs,gaps),default=None),contact_bounds=[segments.reshape(-1,3).min(axis=0).tolist(),segments.reshape(-1,3).max(axis=0).tolist()] if len(segments) else None))
  sections=[];triangles=np.concatenate(alltriangles)
  for z in [20,40,60,80,100]:
   segments=intersections(triangles,np.array([0,0,z]),np.array([0,0,1]));extent=np.ptp(segments.reshape(-1,3),axis=0);sections.append(dict(z=z,width=float(extent[0]),depth=float(extent[1]),ratio=float(extent[1]/extent[0])))
  required={'building-081','building-082'} if args.tree==32 else {'building-094'}
  roots=[r for r in rows if r['source_node'] in required]
  passed=len(roots)==len(required) and all(r['contact_contour_length']>=1 and not r['footprint_misses'] and r['maximum_receiver_plane_gap']<.001 for r in roots) and min(r['ratio'] for r in sections)>=1
  report=dict(status='PASS' if passed else 'HOLD',model_sha256=digest,receiver=str(receiver),receiver_sha256=expected,root_contacts=rows,horizontal_sections=sections,limitations=['Contact uses physical solid wood faces, including inferred bark surfaces; native appearance is a separate saved-source guard.','Receiver is the actual pinned ground mesh; other foliage and canopy do not count as support.','This measures contact contours and main-stem depth, not buried volume or full-scene source parity.'])
  if sha(model)!=digest or sha(receiver)!=expected:raise ValueError('Read-only input changed')
  args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
  if not passed:raise ValueError('Independent root/receiver or thickness guard failed')
 finally:release()
if __name__=='__main__':main()
