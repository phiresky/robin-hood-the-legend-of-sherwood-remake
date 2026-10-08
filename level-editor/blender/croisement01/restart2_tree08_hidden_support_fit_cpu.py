"""Fit bounded back depth so added underside geometry remains behind receivers."""
import hashlib,json
import numpy as np
from scipy.optimize import linprog
from scipy.sparse import coo_matrix
from restart2_tree08_root_embedding_cpu import R,P,mesh
from restart2_tree08_junction_proof import native_depth
from restart2_tree08_local_fork import audit
O=R/'tree08-hidden-support-fit-cpu-v1';O.mkdir(exist_ok=False);m=np.load(R/'tree08-hidden-back-cpu-v3/candidate.npz');v=m['vertices'];f=m['faces'];before=m['before_vertices'];root=np.load(R/'tree08-root-ray-cpu-v4/candidate.npz');s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0.,-c,s]);xy=np.c_[v[:,0]-331,-v[:,1]*s-v[:,2]*c-11];depth=v@ray;receivers=[]
for b in json.loads((P/'current-contact/receipt.json').read_text())['bindings']:
 rv,rf=mesh(b['asset']);rv+=b['translation'];receivers.append((rv,rf))
base_depth=native_depth([(root['vertices'],root['faces'])]);receiver=native_depth(receivers);current=native_depth([(v,f)]);new=~np.isfinite(base_depth)&np.isfinite(current);yy,xx=np.where(new);moved=np.any(v!=before,axis=1);ids=np.flatnonzero(moved);index={int(i):n for n,i in enumerate(ids)};triples=[];rhs=[];tri=xy[f];lo=tri.min(1);hi=tri.max(1);a=tri[:,0];b=tri[:,1]-a;cc=tri[:,2]-a;det=b[:,0]*cc[:,1]-b[:,1]*cc[:,0];constraints=[]
for y,x in zip(yy,xx):
 point=np.array([x+.5,y+.5]);candidate=np.flatnonzero((lo[:,0]<=point[0]+1e-8)&(hi[:,0]>=point[0]-1e-8)&(lo[:,1]<=point[1]+1e-8)&(hi[:,1]>=point[1]-1e-8)&(abs(det)>1e-10));support=receiver[y,x];assert np.isfinite(support)
 for j in candidate:
  delta=point-a[j];u=(delta[0]*cc[j,1]-delta[1]*cc[j,0])/det[j];vv=(b[j,0]*delta[1]-b[j,1]*delta[0])/det[j];weights=np.array([1-u-vv,u,vv])
  if weights.min()<-1e-8:continue
  deficit=float(weights@depth[f[j]]-support+.1)
  if deficit<=0:continue
  row=len(rhs);rhs.append(-deficit)
  for vi,w in zip(f[j],weights):
   if int(vi) in index:triples.append((row,index[int(vi)],-float(w)))
  constraints.append({'native':[int(x)+331,int(y)+11],'face':int(j),'required_depth':deficit})
# Limit adjacent thickness gradients and total inferred depth instead of accepting a sharp spike.
edges=np.unique(np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1),axis=0);edges=edges[np.any(moved[edges],axis=1)]
for a,b in edges:
 limit=4*float(np.linalg.norm(before[a]-before[b]))
 for sign in [-1,1]:
  row=len(rhs);rhs.append(limit)
  if int(a) in index:triples.append((row,index[int(a)],sign))
  if int(b) in index:triples.append((row,index[int(b)],-sign))
rows,cols,data=zip(*triples);matrix=coo_matrix((data,(rows,cols)),shape=(len(rhs),len(ids))).tocsr();fit=linprog(np.ones(len(ids)),A_ub=matrix,b_ub=rhs,bounds=(0,40),method='highs');report={'status':'LP_PASS_GEOMETRY_GUARDS_PENDING' if fit.success else 'NO_BOUNDED_SOLUTION','message':fit.message,'variables':len(ids),'visibility_constraints':len(constraints),'constraints':len(rhs),'maximum_allowed_depth':40,'maximum_allowed_edge_gradient':4,'source_facing_vertices_fixed':True,'new_projected_pixels':len(xx),'constraint_evidence':constraints}
if fit.success:
 delta=np.zeros(len(v));delta[ids]=fit.x;after=v-delta[:,None]*ray;origin=np.array([552.,-672.,235.]);after=(after-origin).astype(np.float32).astype(float)+origin;after[delta==0]=v[delta==0];after_depth=native_depth([(after,f)]);composite=np.maximum(after_depth,receiver);reference=np.maximum(base_depth,receiver);report.update(maximum_added_depth=float(delta.max()),topology=audit(after,f),source_first_hit_error=float(abs(after_depth[np.isfinite(base_depth)]-base_depth[np.isfinite(base_depth)]).max()),composite_error=float(abs(composite-reference)[np.isfinite(reference)].max()));np.savez_compressed(O/'candidate.npz',vertices=after,faces=f,before_vertices=before);report['candidate_sha256']=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest()
(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='constraint_evidence'},indent=2))
