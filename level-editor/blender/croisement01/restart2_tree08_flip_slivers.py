"""Retriangulate two coplanar faces without moving their surface or vertices."""
import json
from collections import defaultdict
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[2]/'work/croisement01-refinement/restart2'
source=R/'tree08-v12-distinct-stems-clearance-v1-stitched';out=source.with_name(source.name+'-flipped');out.mkdir(exist_ok=False)
m=np.load(source/'candidate.npz');v=m['vertices'];f=m['faces'].copy();changes=[]
tri=v[f];area=np.linalg.norm(np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]),axis=1)
for i in np.where(area<1e-9)[0]:
 edges=defaultdict(list)
 for k,face in enumerate(f):
  for j in range(3):edges[tuple(sorted([int(face[j]),int(face[(j+1)%3])]))].append(k)
 accepted=False
 for j in np.argsort(-np.linalg.norm(np.roll(v[f[i]],-1,axis=0)-v[f[i]],axis=1)):
  x,y,z=map(int,np.roll(f[i],-int(j)));other=[k for k in edges[tuple(sorted([x,y]))] if k!=i]
  if len(other)!=1:continue
  k=other[0];w=next(int(p) for p in f[k] if p not in [x,y]);t=v[f[k]];n=np.cross(t[1]-t[0],t[2]-t[0]);n/=np.linalg.norm(n)
  distance=float(np.max(abs((v[f[i]]-t[0])@n)))
  if distance>1e-11:continue
  candidate=np.array([[z,x,w],[z,w,y]]);ct=v[candidate];normals=np.cross(ct[:,1]-ct[:,0],ct[:,2]-ct[:,0]);new_area=np.linalg.norm(normals,axis=1)
  if np.min(normals@n)<=1e-9:continue
  old_area=area[i]+area[k];error=abs(float(new_area.sum()-old_area))
  if error>1e-10:continue
  changes.append(dict(faces=[int(i),int(k)],original=f[[i,k]].tolist(),replacement=candidate.tolist(),maximum_coplanarity_distance=distance,area_error=error,minimum_new_cross_norm=float(new_area.min())))
  f[[i,k]]=candidate;accepted=True;break
 assert accepted,dict(face=int(i),reason='No exact coplanar convex diagonal replacement')
np.savez_compressed(out/'candidate.npz',vertices=v,faces=f)
(out/'report.json').write_text(json.dumps(dict(status='Topology only; full source/intersection proof pending',changes=changes,vertex_positions_unchanged=True,topology=audit(v,f)),indent=2)+'\n')
print((out/'report.json').read_text())
