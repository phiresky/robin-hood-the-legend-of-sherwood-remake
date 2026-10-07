"""Repair serialization-sensitive local slivers with fixed-position diagonal flips."""
import argparse,json
from collections import defaultdict
from pathlib import Path
import numpy as np
from restart2_tree08_local_fork import audit
ORIGIN=np.array([552.,-672.,235.])
def normals(v,f):
 t=v[f];n=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]);return n,np.linalg.norm(n,axis=1)
def alignment(v,q,f):
 n,a=normals(v,f);o,b=normals(q,f);return (n*o).sum(1)/np.maximum(a*b,1e-100)
def main():
 parser=argparse.ArgumentParser();parser.add_argument('packet',type=Path);args=parser.parse_args();source=args.packet;out=source.with_name(source.name+'-stable');out.mkdir(exist_ok=False)
 m=np.load(source/'candidate.npz');v=m['vertices'];f=m['faces'].copy();q=(v-ORIGIN).astype(np.float32).astype(np.float64)+ORIGIN;changes=[]
 initial=alignment(v,q,f)
 for iteration in range(5):
  scores=alignment(v,q,f);bad=np.where(scores<.99)[0]
  if not len(bad):break
  changed=0
  edges=defaultdict(list)
  for k,face in enumerate(f):
   for j in range(3):edges[tuple(sorted([int(face[j]),int(face[(j+1)%3])]))].append(k)
  touched=set()
  for i in bad:
   if int(i) in touched:continue
   candidates=[]
   for j in range(3):
    x,y,z=map(int,np.roll(f[i],-j));neighbors=edges[tuple(sorted([x,y]))]
    if len(neighbors)!=2:continue
    k=next(k for k in neighbors if k!=i)
    if k in touched:continue
    w=next(int(p) for p in f[k] if p not in [x,y])
    if z==w or tuple(sorted([z,w])) in edges:continue
    t=v[f[k]];n=np.cross(t[1]-t[0],t[2]-t[0]);length=np.linalg.norm(n)
    if length<1e-9:continue
    n/=length;distance=float(np.max(abs((v[f[i]]-t[0])@n)))
    if distance>1e-6:continue
    candidate=np.array([[z,x,w],[z,w,y]]);ns,areas=normals(v,candidate)
    if np.min(ns@n)<=1e-9:continue
    score=float(alignment(v,q,candidate).min())
    if score<.999:continue
    candidates.append((score,float(areas.min()),k,candidate,distance))
   if not candidates:continue
   score,area,k,candidate,distance=max(candidates,key=lambda c:(c[0],c[1]))
   changes.append(dict(faces=[int(i),int(k)],original=f[[i,k]].tolist(),replacement=candidate.tolist(),maximum_local_surface_plane_distance=distance,minimum_saved_normal_alignment=score,minimum_new_cross_norm=area))
   f[[i,k]]=candidate;touched.update([int(i),int(k)]);changed+=1
  print('PASS',iteration,'unstable',len(bad),'flips',changed,flush=True)
  if not changed:break
 final=alignment(v,q,f);report=dict(status='CPU triangulation candidate; source and intersection guards pending',changes=changes,vertex_positions_unchanged=True,initial_unstable=int((initial<.99).sum()),remaining_unstable=np.where(final<.99)[0].tolist(),minimum_saved_normal_alignment=float(final.min()),maximum_quantization_displacement=float(np.linalg.norm(v-q,axis=1).max()),topology=audit(v,f),saved_topology=audit(q,f),surface_guard='Only local nearly planar convex quadrilaterals; measured surface plane departure <=1e-6, fixed vertex positions, global2e-4/source-pixel guards still mandatory.')
 np.savez_compressed(out/'candidate.npz',vertices=v,faces=f);(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print({k:x for k,x in report.items() if k!='changes'},flush=True)
if __name__=='__main__':main()
