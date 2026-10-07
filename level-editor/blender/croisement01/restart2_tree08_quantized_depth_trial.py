"""Search neighboring representable coordinates for one inferred grazing-ray outlier."""
import json,itertools
from pathlib import Path
import numpy as np
R=Path(__file__).resolve().parents[2]/'work/croisement01-refinement/restart2';p=R/'tree08-v12-remaining-group0-stitched-v3-conformed-stable';m=np.load(p/'candidate.npz');v=m['vertices'];f=m['faces'];origin=np.array([552.,-672.,235.]);local=(v-origin).astype(np.float32);q=local.astype(float)+origin;s,c=np.sin(np.radians(35)),np.cos(np.radians(35));target=np.array([497.5,96.5]);reference=json.loads((p/'depth-outliers.json').read_text())['outliers'][0]['reference']
def samples(vertices,faces):
 t=vertices[faces];xy=np.stack([t[:,:,0],-t[:,:,1]*s-t[:,:,2]*c],axis=2);a,b,d=xy[:,0],xy[:,1]-xy[:,0],xy[:,2]-xy[:,0];det=b[:,0]*d[:,1]-b[:,1]*d[:,0];safe=abs(det)>1e-10;delta=target-a;u=np.divide(delta[:,0]*d[:,1]-delta[:,1]*d[:,0],det,where=safe,out=np.zeros_like(det));w=np.divide(b[:,0]*delta[:,1]-b[:,1]*delta[:,0],det,where=safe,out=np.zeros_like(det));valid=safe&(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8);depth=-t[:,:,1]*c+t[:,:,2]*s;result=depth[:,0]+u*(depth[:,1]-depth[:,0])+w*(depth[:,2]-depth[:,0]);result[~valid]=-np.inf;return result
base=samples(q,f);owner=int(np.argmax(base));affected_vertices=list(map(int,f[owner]));candidates=[]
for vertex in affected_vertices:
 original=local[vertex].copy();adjacent=np.where(np.any(f==vertex,axis=1))[0];other=float(np.max(np.delete(base,adjacent)))
 for delta in itertools.product([-1,0,1],repeat=3):
  if delta==(0,0,0):continue
  value=original.copy()
  for axis,step in enumerate(delta):
   if step:value[axis]=np.nextafter(value[axis],np.float32(np.inf if step>0 else -np.inf))
  q[vertex]=value.astype(float)+origin;depth=max(other,float(samples(q,f[adjacent]).max()));error=abs(depth-reference);distance=float(np.linalg.norm(q[vertex]-v[vertex]))
  if error<=2e-4 and distance<=2e-4:
   old=v[f[adjacent]];new=q[f[adjacent]];on=np.cross(old[:,1]-old[:,0],old[:,2]-old[:,0]);nn=np.cross(new[:,1]-new[:,0],new[:,2]-new[:,0]);align=(on*nn).sum(1)/(np.linalg.norm(on,axis=1)*np.linalg.norm(nn,axis=1))
   if float(align.min())>=.99:candidates.append(dict(vertex=vertex,ulp_steps=delta,world=q[vertex].tolist(),maximum_vertex_displacement=distance,depth_error=error,minimum_adjacent_normal_alignment=float(align.min())))
  q[vertex]=original.astype(float)+origin
candidates.sort(key=lambda x:(x['maximum_vertex_displacement'],x['depth_error']));report=dict(status='CPU trial only; no mutation to candidate',native=[497,96],face=owner,vertices=affected_vertices,reference_depth=reference,original_saved_depth=float(base[owner]),candidates=candidates)
(p/'quantized-depth-trial.json').write_text(json.dumps(report,indent=2)+'\n');print(dict(face=owner,vertices=affected_vertices,candidates=len(candidates),best=candidates[:2]))
