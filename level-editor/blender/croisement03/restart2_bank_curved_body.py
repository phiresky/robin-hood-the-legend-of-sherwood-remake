"""Refine both closed bank bodies while preserving every authored source crease."""
import json,hashlib,math
from pathlib import Path
from collections import defaultdict,Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'bank-curved-body-cpu-v1';ray=np.array([0.,-math.cos(math.radians(35)),math.sin(math.radians(35))])
def main():
 O.mkdir(exist_ok=False);input_=B/'bank-sidewall-cpu-v1/geometry.json';data=json.loads(input_.read_text());report=[]
 for owner in ['52','54']:
  mesh=data[owner];verts=np.array(mesh['vertices']);faces=mesh['faces'];n=mesh['source_vertex_count'];top=[all(i<n for i in f)for f in faces];edges=defaultdict(list)
  for fi,f in enumerate(faces):
   for a,b,c in [(f[0],f[1],f[2]),(f[1],f[2],f[0]),(f[2],f[0],f[1])]:edges[tuple(sorted((a,b)))].append((fi,c))
  segments=[(verts[a],verts[b])for trace in mesh['traces']for a,b in zip(trace['vertices'],trace['vertices'][1:])]
  def on_segment(p,a,b):
   d=b-a;t=float(np.dot(p-a,d)/np.dot(d,d));return -1e-6<=t<=1+1e-6 and np.linalg.norm(p-(a+t*d))<1e-4
  points=verts.tolist();mids={};sharp=[];moved=[]
  for edge,adjacent in edges.items():
   assert len(adjacent)==2;a,b=edge;mid=(verts[a]+verts[b])/2;constrained=any(on_segment(verts[a],p,q)and on_segment(verts[b],p,q)for p,q in segments);interior_top=all(top[fi]for fi,_ in adjacent)
   amount=0.
   if interior_top and not constrained:
    opposites=[verts[c]for _,c in adjacent];smooth=.375*(verts[a]+verts[b])+.125*(opposites[0]+opposites[1]);amount=float(np.clip(np.dot(smooth-mid,ray),-2.,0.));mid+=ray*amount
   index=len(points);points.append(mid.tolist());mids[edge]=index
   if constrained or not interior_top:sharp += [[a,index],[index,b]]
   if abs(amount)>1e-7:moved.append({'edge':list(edge),'new_vertex':index,'native_ray_shift':amount})
  newfaces=[];parents=[]
  for fi,(a,b,c)in enumerate(faces):
   ab=mids[tuple(sorted((a,b)))];bc=mids[tuple(sorted((b,c)))];ca=mids[tuple(sorted((c,a)))];newfaces += [[a,ab,ca],[ab,b,bc],[ca,bc,c],[ab,bc,ca]];parents += [fi]*4
  counts=Counter(tuple(sorted((a,b)))for f in newfaces for a,b in zip(f,f[1:]+f[:1]));assert set(counts.values())=={2};assert len(points)-len(counts)+len(newfaces)==2;volume=sum(float(np.dot(points[f[0]],np.cross(points[f[1]],points[f[2]])))/6 for f in newfaces);assert volume>0
  mesh.update(vertices=points,faces=newfaces,signed_volume=volume,edge_count=len(counts),euler=2,source_parent_faces=parents,sharp_edges=sharp,topology='Closed curved body with exact old vertices/crease midpoints and bounded inward relief; no raised anchors.');report.append({'owner':owner,'retained_old_vertices':len(verts),'new_vertices':len(points)-len(verts),'curved_edges':len(moved),'max_inward_ray_shift':max([-m['native_ray_shift']for m in moved],default=0),'outward_or_height_inflation':False,'controls':moved})
 data['body_revision']={'input_sha256':hashlib.sha256(input_.read_bytes()).hexdigest(),'owners':report};data['scope']='Private complete52/54 body candidate retaining source strata and sidewall corrections; saved source/neighbor/appearance review pending.';(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print([{k:v for k,v in row.items()if k!='controls'}for row in report])
if __name__=='__main__':main()
