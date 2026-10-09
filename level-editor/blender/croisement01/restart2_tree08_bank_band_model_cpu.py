"""Fit unapproved root depths against real receivers with an injective band shear."""
import hashlib,json,math,sys
from pathlib import Path
import numpy as np
from PIL import Image
from restart2_tree08_junction_proof import native_depth
from restart2_tree08_local_fork import audit
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';out=R/'tree08-bank-hug-affine-cpu-v2';out.mkdir(exist_ok=False);parent=np.load(R/'tree08-root-ray-cpu-v4/candidate.npz');original=parent['vertices'];oldfaces=parent['faces'];s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0,-c,s]);down=np.array([0,-s,-c]);origin=np.array([552.,-672.,235.]);yn=-original[:,1]*s-original[:,2]*c
fit=json.loads((R/'tree08-bank-band-fit-cpu-v1/report.json').read_text());fit_knots=np.array(fit['knots_native_y_ray_delta']) if fit else None;wanted=fit_knots[:,0].tolist();planes=[]
for target in wanted:
 nearby=sorted(set([target-.04,target+.04]+[float(x) for x in yn if target-.04<x<target+.04]));lo,hi=max(zip(nearby,nearby[1:]),key=lambda pair:pair[1]-pair[0]);planes.append((lo+hi)/2)
vertices=original.tolist();native_y=yn.tolist();cache={};faces=[];parents=[]
def crossing(a,b,k):
 key=(min(a,b),max(a,b),k)
 if key in cache:return cache[key]
 a,b=key[:2];t=(planes[k]-native_y[a])/(native_y[b]-native_y[a]);assert 0<t<1;p=np.array(vertices[a])+t*(np.array(vertices[b])-vertices[a]);index=len(vertices);vertices.append(p.tolist());native_y.append(planes[k]);cache[key]=index;return index
def cut(poly,k,below):
 result=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  da=native_y[a]-planes[k];db=native_y[b]-planes[k];inside=da<=0 if below else da>=0
  if inside:result.append(a)
  if da*db<0:result.append(crossing(a,b,k))
 return result
for i,triangle in enumerate(oldfaces):
 pieces=[list(map(int,triangle))];low,high=min(yn[triangle]),max(yn[triangle])
 for k,plane in enumerate(planes):
  if not low<plane<high:continue
  nextpieces=[]
  for poly in pieces:
   vals=[native_y[v] for v in poly]
   if min(vals)<plane<max(vals):nextpieces.extend([cut(poly,k,True),cut(poly,k,False)])
   else:nextpieces.append(poly)
  pieces=nextpieces
 for poly in pieces:
  for j in range(1,len(poly)-1):faces.append((poly[0],poly[j],poly[j+1]));parents.append(i)
v=np.array(vertices);f=np.array(faces,dtype=np.int32);native_y=np.array(native_y);before_audit=audit(v,f);assert not any(before_audit[k] for k in ['nonmanifold_edges','winding_errors','zero_area']),before_audit
heights=np.interp(planes,*fit_knots.T).tolist() if fit else [0,50,75,75,65,43.5,43.5,40];delta=np.interp(native_y,planes,heights,left=0,right=heights[-1]);extension=np.zeros(len(native_y));candidate=v+delta[:,None]*ray+extension[:,None]*down;candidate=(candidate-origin).astype(np.float32).astype(float)+origin;check=audit(candidate,f)
if check['zero_area']:
 t=candidate[f];bad=np.linalg.norm(np.cross(t[:,1]-t[:,0],t[:,2]-t[:,0]),axis=1)<1e-10;np.savez_compressed(out/'failed-mesh.npz',vertices=candidate,faces=f,before_vertices=v);(out/'degeneracy.json').write_text(json.dumps({'faces':np.flatnonzero(bad).tolist(),'positions':t[bad].tolist()},indent=2)+'\n')
assert not any(check[k] for k in ['nonmanifold_edges','winding_errors','zero_area']),check
before=native_depth([(original,oldfaces)]);after=native_depth([(candidate,f)]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;assert np.isfinite(after[core]).all();depth_error=float(abs(before[core]-after[core]).max());assert depth_error<=.0002;lost=np.isfinite(before)&~np.isfinite(after);gained=~np.isfinite(before)&np.isfinite(after);assert not lost.any();gy,gx=np.where(gained);assert not gained.any();assert np.isfinite(after[458,247]);assert np.array_equal(candidate[native_y<=320],v[native_y<=320]);np.savez_compressed(out/'candidate.npz',vertices=candidate,faces=f,before_vertices=v,parent_faces=np.array(parents,dtype=np.int32))
report=dict(status='CPU_AFFINE_ROOT_HYPOTHESIS_PENDING_INTERSECTION_GUARD',model_parent_sha256=hashlib.sha256((R/'tree08-wood-prototype-v14-root-ray/model.blend').read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),knots_native_y_ray_delta=list(zip(planes,heights)),terminal_extension=0,inserted_vertices=len(v)-len(original),added_triangles=len(f)-len(oldfaces),topology_before=before_audit,topology_after=check,source_core_pixels=6276,source_core_depth_error=depth_error,source_core_position_fixed=True,silhouette_lost=int(lost.sum()),silhouette_gained=int(gained.sum()),gained_native_pixels=[[int(x)+331,int(y)+11] for y,x in np.argwhere(gained)],terminal_E_pixel_hit=True,method='Shared edge/plane intersections; same affine native-ray shear on every triangle in each band. No remesh, source upper geometry fixed.',limitations=['Lower-root depths move to maintain actual bank contact; prior inferred depth is not observed source authority.','Receiver unchanged; root embedding and source material boundaries still require independent review.','Existing upper branch ridges are not corrected.'])
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
