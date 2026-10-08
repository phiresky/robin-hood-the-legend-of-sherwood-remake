"""Conform a local piecewise-affine monotone back-depth extension before deformation."""
import hashlib,json,sys
import numpy as np
from PIL import Image
from restart2_tree08_root_embedding_cpu import R
from restart2_tree08_local_fork import audit
from restart2_tree08_junction_proof import native_depth
revision=4 if '--contact-right' in sys.argv else 3 if '--contact-left' in sys.argv else 2 if '--descending' in sys.argv else 1;stable='--stable-cuts' in sys.argv;O=R/(f'tree08-monotone-back-cpu-v{revision}'+('-stable' if stable else ''));O.mkdir(exist_ok=False);source=R/(f'tree08-monotone-back-cpu-v{revision-1}'+('-stable' if stable and revision==4 else '')+'/candidate.npz' if revision>1 else 'tree08-root-ray-cpu-v4/candidate.npz');m=np.load(source);original=m['vertices'];faces=m['faces'];ray=np.array([0,-np.cos(np.radians(35)),np.sin(np.radians(35))]);down=np.array([0,-np.sin(np.radians(35)),-np.cos(np.radians(35))]);field=json.loads((R/('tree08-contact-field-fit-cpu-v2/report.json' if revision>=3 else 'tree08-monotone-field-plan-cpu-v1/report.json')).read_text())['fields'][1 if revision in [2,4] else 0];x0,y0,x1,y1=field['native_box'];a,b,c=field['threshold_plane']
if stable:
 old_mx,old_my=(x0+x1)/2,(y0+y1)/2;x0+=.017;x1-=.013;y0+=.019;y1-=.011;c+=a*((x0+x1)/2-old_mx)+b*((y0+y1)/2-old_my)-.004;field=dict(field,native_box=[x0,y0,x1,y1],threshold_plane=[a,b,c],stable_cut_note='Box shrunk by at most.019 and threshold lowered.004, within.25 contact margin; exact thresholds unchanged.')
coeff=np.array([8*(np.array([a,0,0])+b*down-ray),[6,0,0],[-6,0,0],6*down,-6*down,[0,0,0]],float);constant=np.array([8*(c-a*(x0+x1)/2-b*(y0+y1)/2),-6*x0,6*x1,-6*y0,6*y1,40]);planes=[(coeff[i],constant[i]) for i in range(5)]+[(coeff[i]-coeff[j],constant[i]-constant[j]) for i in range(6) for j in range(i+1,6)];pool=original.tolist();support=[frozenset([i]) for i in range(len(pool))];cache={};edgecuts={};pieces={}
def signed(index,k):return float(np.array(pool[index])@planes[k][0]+planes[k][1])
def cross(u,v,k):
 su,sv=signed(u,k),signed(v,k)
 if abs(su)<1e-10:return u
 if abs(sv)<1e-10:return v
 shared=support[u]|support[v];edge=tuple(sorted(shared)) if len(shared)==2 else None;key=('original',edge,k) if edge else ('segment',min(u,v),max(u,v),k)
 if key in cache:return cache[key]
 fraction=su/(su-sv);assert 0<fraction<1;point=np.array(pool[u])+fraction*(np.array(pool[v])-pool[u]);index=len(pool);pool.append(point.tolist());support.append(shared);cache[key]=index
 if edge is not None:
  start,end=original[list(edge)];t=float((point-start)@(end-start)/((end-start)@(end-start)));edgecuts.setdefault(edge,{})[index]=t
 return index
def clip(poly,k,positive):
 result=[]
 for u,v in zip(poly,poly[1:]+poly[:1]):
  su,sv=signed(u,k),signed(v,k)
  if su>=-1e-10 if positive else su<=1e-10:result.append(u)
  if su*sv<0 and abs(su)>1e-10 and abs(sv)>1e-10:result.append(cross(u,v,k))
 return list(dict.fromkeys(result))
values=original@coeff.T+constant;selected=np.all(np.max(values[faces],axis=1)>0,axis=1)
for fi in np.flatnonzero(selected):
 polys=[list(map(int,faces[fi]))]
 for k in range(len(planes)):
  output=[]
  for polygon in polys:
   d=[signed(i,k) for i in polygon]
   if min(d)<-1e-10 and max(d)>1e-10:
    output.extend([clip(polygon,k,True),clip(polygon,k,False)])
   else:output.append(polygon)
  polys=output
 pieces[int(fi)]=polys
# Insert every shared-edge cut in neighbors too; never leave a geometric T-junction.
conformed=[]
for fi,face in enumerate(faces):
 for poly in pieces.get(fi,[list(map(int,face))]):
  boundary=[]
  for u,v in zip(poly,poly[1:]+poly[:1]):
   boundary.append(u);shared=support[u]|support[v]
   if len(shared)!=2:continue
   edge=tuple(sorted(shared));cuts=edgecuts.get(edge,{})
   if not cuts:continue
   start,end=original[list(edge)];vector=end-start;den=vector@vector;tu=float((np.array(pool[u])-start)@vector/den);tv=float((np.array(pool[v])-start)@vector/den)
   between=[(value,index) for index,value in cuts.items() if min(tu,tv)+1e-10<value<max(tu,tv)-1e-10];between.sort(reverse=tu>tv);boundary.extend(i for _,i in between)
  boundary=list(dict.fromkeys(boundary))
  if len(boundary)==3:conformed.append(tuple(boundary));continue
  center=len(pool);pool.append(np.array([pool[i] for i in boundary]).mean(0).tolist());support.append(frozenset())
  for u,v in zip(boundary,boundary[1:]+boundary[:1]):conformed.append((u,v,center))
v=np.array(pool);f=np.array(conformed,np.int32);origin=np.array([552.,-672.,235.]);v[len(original):]=(v[len(original):]-origin).astype(np.float32).astype(float)+origin;before_topology=audit(v,f);assert not any(before_topology[k] for k in ['nonmanifold_edges','winding_errors','zero_area']),before_topology
amount=np.maximum(0,np.min(v@coeff.T+constant,axis=1));after=v-amount[:,None]*ray;after=(after-origin).astype(np.float32).astype(float)+origin;after[amount==0]=v[amount==0];topology=audit(after,f);prior=native_depth([(original,faces)]);current=native_depth([(after,f)]);finite=np.isfinite(prior);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;error=float(abs(current[finite]-prior[finite]).max());lost=int((finite&~np.isfinite(current)).sum());gained=int((~finite&np.isfinite(current)).sum());np.savez_compressed(O/'candidate.npz',vertices=after,faces=f,before_vertices=v);report={'status':'CPU MONOTONE FIELD; STRICT INTERSECTION AND CONTACT GUARDS PENDING','candidate_sha256':hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),'parent_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'field':field,'selected_original_faces':int(selected.sum()),'inserted_vertices':len(v)-len(original),'added_triangles':len(f)-len(faces),'moved_vertices':int((amount>0).sum()),'maximum_extension':float(amount.max()),'core_pixels':int(core.sum()),'core_depth_error':float(abs(current[core]-prior[core]).max()),'all_first_hit_depth_error':error,'silhouette_lost':lost,'silhouette_gained':gained,'topology':topology,'before_topology':before_topology,'construction':'Shared plane cuts, conformed edge subdivisions; monotone native-depth derivative1 or9. Local box ramps and40-unit maximum. Upper source core lies outside the field.'};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({k:v for k,v in report.items() if k!='field'},indent=2));assert not any(topology[k] for k in ['nonmanifold_edges','winding_errors','zero_area']);assert error<=.0002 and not lost and not gained
