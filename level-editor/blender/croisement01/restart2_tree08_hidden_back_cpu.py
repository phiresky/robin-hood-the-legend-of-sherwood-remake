"""Test connected inferred root back extension with all facing triangles fixed."""
import hashlib,json,sys
from pathlib import Path
import numpy as np
from scipy.sparse import coo_matrix,diags
from scipy.sparse.csgraph import dijkstra
from PIL import Image
from restart2_tree08_root_embedding_cpu import R,P,mesh,triangles,heights
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import native_depth
O=R/('tree08-hidden-back-cpu-v2' if '--subdivide' in sys.argv else 'tree08-hidden-back-cpu-v1');O.mkdir(exist_ok=False);source=R/'tree08-root-ray-cpu-v4/candidate.npz';m=np.load(source);v=m['vertices'];f=m['faces'];ray=np.array([0,-np.cos(np.radians(35)),np.sin(np.radians(35))]);down=np.array([0,-np.sin(np.radians(35)),-np.cos(np.radians(35))]);direction=-ray;base_v=v.copy();base_f=f.copy()
if '--subdivide' in sys.argv:
 # Share every split edge with its neighbor, including unchanged front-facing triangles.
 original_count=len(v);pool=v.tolist();midpoints={};newfaces=[];split_faces=np.max(v[f]@down,axis=1)>360
 for face in f[split_faces]:
  for a,b in zip(face,np.roll(face,-1)):
   key=tuple(sorted((int(a),int(b))))
   if key not in midpoints:midpoints[key]=len(pool);pool.append(((v[a]+v[b])*.5).tolist())
 for face in f:
  polygon=[]
  for a,b in zip(face,np.roll(face,-1)):
   polygon.append(int(a));midpoint=midpoints.get(tuple(sorted((int(a),int(b)))))
   if midpoint is not None:polygon.append(midpoint)
  if len(polygon)==3:newfaces.append(tuple(polygon));continue
  center=len(pool);pool.append(v[face].mean(0).tolist())
  for a,b in zip(polygon,polygon[1:]+polygon[:1]):newfaces.append((a,b,center))
 v=np.array(pool);f=np.array(newfaces,dtype=np.int32);origin=np.array([552.,-672.,235.]);v[original_count:]=(v[original_count:]-origin).astype(np.float32).astype(float)+origin
t=v[f];n=np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]);front=(n@ray)>=-1e-8;fixed=np.zeros(len(v),bool);fixed[f[front].ravel()]=True;yn=v@down;eligible=(~fixed)&(yn>360)
receivers=[]
for binding in json.loads((P/'current-contact/receipt.json').read_text())['bindings']:
 rv,rf=mesh(binding['asset']);rv+=binding['translation'];receivers.append((binding['asset']['id'],rv[rf],triangles(rv,rf)))
def first_hit(origin,t):
 e1=t[:,1]-t[:,0];e2=t[:,2]-t[:,0];h=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,h);ok=abs(det)>1e-10;inv=np.where(ok,1/np.where(ok,det,1),0);s=origin-t[:,0];u=inv*np.einsum('ij,ij->i',s,h);q=np.cross(s,e1);b=inv*(q@direction);distance=inv*np.einsum('ij,ij->i',e2,q);ok&=(u>=-1e-8)&(b>=-1e-8)&(u+b<=1+1e-8)&(distance>=0)
 return float(distance[ok].min()) if ok.any() else None
# Surface distance from fixed facing boundary gives a continuous transition to unchanged outer contours.
edges=np.unique(np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1),axis=0);weights=np.linalg.norm(v[edges[:,0]]-v[edges[:,1]],axis=1);graph=coo_matrix((np.r_[weights,weights],(np.r_[edges[:,0],edges[:,1]],np.r_[edges[:,1],edges[:,0]])),shape=(len(v),len(v))).tocsr();border=np.unique(edges[np.any(fixed[edges],axis=1)&np.any(eligible[edges],axis=1)]);border=border[fixed[border]];distance=dijkstra(graph,directed=False,indices=border,min_only=True);targets=np.zeros(len(v));requested=[]
for i in np.flatnonzero(eligible):
 supports=[heights(packet,v[i,:2]) for _,_,packet in receivers];supports=[float(x.max()) for x in supports if len(x)];assert supports;gap=v[i,2]-max(supports)
 if gap<=.05:continue
 hits=[first_hit(v[i],packet) for _,packet,_ in receivers];hits=[x for x in hits if x is not None];assert hits;depth=min(hits)+.25;fade=np.clip(distance[i]/1.5,0,1);fade=fade*fade*(3-2*fade);targets[i]=depth*fade;requested.append({'vertex':int(i),'vertical_gap':gap,'depth_extension':depth,'fade':float(fade)})
# The proposal never changes front vertices or native projection; strict intersection guard still required.
after=v-targets[:,None]*ray;origin=np.array([552.,-672.,235.]);after=(after-origin).astype(np.float32).astype(float)+origin;after[targets==0]=v[targets==0];assert np.array_equal(after[fixed],v[fixed]);topology=audit(after,f);before_depth=native_depth([(base_v,base_f)]);after_depth=native_depth([(after,f)]);finite=np.isfinite(before_depth);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;error=float(abs(before_depth[finite]-after_depth[finite]).max());lost=int((finite&~np.isfinite(after_depth)).sum());gained=int((~finite&np.isfinite(after_depth)).sum());report={'status':'CPU CANDIDATE; STRICT INTERSECTION GUARD PENDING','parent_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'moved_vertices':int((targets>0).sum()),'maximum_extension':float(targets.max()),'source_facing_vertices_fixed':int(fixed.sum()),'core_pixels':int(core.sum()),'core_depth_error':float(abs(before_depth[core]-after_depth[core]).max()),'all_first_hit_depth_error':error,'silhouette_lost':lost,'silhouette_gained':gained,'topology':topology,'requested':requested,'limitations':['A root-back depth extension hypothesis, not approved anatomy.','Contact, natural taper and source-facing surface guards must all pass independently.']};np.savez_compressed(O/'candidate.npz',vertices=after,faces=f,before_vertices=v);report['candidate_sha256']=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest();(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:x for k,x in report.items() if k!='requested'},indent=2));assert not any(topology[k] for k in ['nonmanifold_edges','winding_errors','zero_area']);assert error<=.0002 and lost==0 and gained==0
