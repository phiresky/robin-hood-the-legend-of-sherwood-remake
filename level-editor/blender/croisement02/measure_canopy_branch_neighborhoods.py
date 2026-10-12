"""Measure fixed upper-wood neighborhoods against every saved crown pose."""
from pathlib import Path
import json,struct,hashlib
import numpy as np
from scipy.spatial import cKDTree
from audit_canopy_static_clearance import primitives

def triangle_distance(point,tri):
 a,b,c=tri[:,0],tri[:,1],tri[:,2];ab=b-a;ac=c-a;ap=point-a;normal=np.cross(ab,ac);nn=np.einsum('ij,ij->i',normal,normal);dot=np.einsum('ij,ij->i',ap,normal);projection=point-np.divide(dot,nn,out=np.zeros_like(dot),where=nn>1e-20)[:,None]*normal;v=projection-a;aa=np.einsum('ij,ij->i',ab,ab);bb=np.einsum('ij,ij->i',ab,ac);cc=np.einsum('ij,ij->i',ac,ac);va=np.einsum('ij,ij->i',v,ab);vc=np.einsum('ij,ij->i',v,ac);den=aa*cc-bb*bb;u=np.divide(cc*va-bb*vc,den,out=np.zeros_like(den),where=np.abs(den)>1e-20);w=np.divide(aa*vc-bb*va,den,out=np.zeros_like(den),where=np.abs(den)>1e-20);inside=(nn>1e-20)&(u>=-1e-10)&(w>=-1e-10)&(u+w<=1+1e-10);best=np.full(len(tri),np.inf);best[inside]=np.sum((projection[inside]-point)**2,axis=1)
 for x,y in [(a,b),(b,c),(c,a)]:
  edge=y-x;length=np.einsum('ij,ij->i',edge,edge);t=np.clip(np.divide(np.einsum('ij,ij->i',point-x,edge),length,out=np.zeros_like(length),where=length>1e-20),0,1);best=np.minimum(best,np.sum((x+t[:,None]*edge-point)**2,axis=1))
 return np.sqrt(np.maximum(best,0))

def main():
 R=Path.cwd();B=R/'level-editor/work/croisement02-refinement';src=B/'restart99-canopy00-combined-v1/report.json';O=B/'canopy00-branch-neighborhoods-oct12-v1';O.mkdir(exist_ok=False);bundle=json.loads(src.read_text());results=[]
 for member in bundle['assets']:
  path=Path(member['source_model']['path']);raw=path.read_bytes();assert hashlib.sha256(raw).hexdigest()==member['source_model']['sha256'];n=struct.unpack_from('<I',raw,12)[0];doc=json.loads(raw[20:20+n]);data=raw[28+n:]
  def read(i):
   a=doc['accessors'][i];v=doc['bufferViews'][a['bufferView']];dtype={5126:'<f4',5125:'<u4',5123:'<u2',5121:'u1'}[a['componentType']];width={'VEC3':3,'SCALAR':1}[a['type']];size=np.dtype(dtype).itemsize;return np.ndarray((a['count'],width),dtype=dtype,buffer=data,offset=v.get('byteOffset',0)+a.get('byteOffset',0),strides=(v.get('byteStride',size*width),size))
  crowns=[];wood=[]
  for _,_,p,w,_ in primitives(doc,member['placement']['transform'],member['placement'].get('parts')):
   pos=read(p['attributes']['POSITION']);world=pos@w[:3,:3].T+w[:3,3]
   if p.get('targets'):crowns.append((p,w))
   else:wood.append(world)
  wood=np.unique(np.concatenate(wood),axis=0);cutoff=wood[:,1].min()+.65*np.ptp(wood[:,1]);upper=wood[wood[:,1]>=cutoff];bins=np.floor(upper[:,[0,2]]/8).astype(int);chosen={}
  for vertex,key in zip(upper,bins):
   key=tuple(key)
   if key not in chosen or vertex[1]>chosen[key][1]:chosen[key]=vertex
  probes=np.array(list(chosen.values()));distances=[];max_motion=0
  for phase in range(14):
   triangles=[]
   for p,w in crowns:
    base=read(p['attributes']['POSITION']);delta=read(p['targets'][phase-1]['POSITION'])if phase else np.zeros_like(base);max_motion=max(max_motion,float(np.linalg.norm(delta@w[:3,:3].T,axis=1).max()));points=(base+delta)@w[:3,:3].T+w[:3,3];triangles.append(points[read(p['indices']).reshape(-1,3)])
   tri=np.concatenate(triangles);lo=tri.min(1);hi=tri.max(1);vertex_tree=cKDTree(tri.reshape(-1,3));nearest_vertex=vertex_tree.query(probes,workers=1)[0];values=[]
   for point,bound in zip(probes,nearest_vertex):
    lower=np.linalg.norm(np.maximum(np.maximum(lo-point,point-hi),0),axis=1);candidates=tri[lower<=bound+1e-7];assert len(candidates);values.append(float(triangle_distance(point,candidates).min()))
   distances.append(values)
  d=np.asarray(distances);summary=[]
  for radius in [4,8,16,24,32]:
   baseline=d[0]<=radius;summary.append({'radius':radius,'baseline_covered':int(baseline.sum()),'covered_each_phase':(d<=radius).sum(1).tolist(),'baseline_probes_leaving_radius_any_phase':int(np.any(d[:,baseline]>radius,axis=0).sum())})
  near=d[0]<=24;result={'asset':member['asset_id'],'model_pin':member['source_model'],'probe_rule':'Highest fixedwood vertex in each8x8worldXZcell, restricted to upper35%of fixedwood height. These are upper-branch surface neighborhood probes, not inferred welded leaf-card junctions.','probe_count':len(probes),'upper_wood_y_cutoff':float(cutoff),'probes_yup':probes.tolist(),'nearest_crown_triangle_distance_by_phase':d.tolist(),'radius_coverage':summary,'baseline_near24_probe_count':int(near.sum()),'max_distance_increase_for_baseline_near24':float((d[:,near]-d[0,near]).max())if near.any()else None,'max_distance_any_pose_for_baseline_near24':float(d[:,near].max())if near.any()else None,'max_crown_vertex_motion':max_motion};results.append(result);print(member['asset_id'],len(probes),result['max_distance_increase_for_baseline_near24'],summary,flush=True)
 report={'status':'MEASURED_STATIC_BASELINE_NEIGHBORHOODS_ALL14POSES','source':{'path':str(src),'sha256':hashlib.sha256(src.read_bytes()).hexdigest()},'assets':results,'limits':['Actual point-to-triangle geometric distances; texture alpha and hidden twig continuity are not inferred from leaf cards.','Fixed thresholds report coverage changes, not automatically pass/fail botanical attachment. Assess motion-relative differences with saved diagnostic views.','Only canopy00trees43/45/46. Upperwood sample is explicit, not a claim of exhaustive anatomical branch-tip detection.']};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('sha',hashlib.sha256((O/'report.json').read_bytes()).hexdigest())
if __name__=='__main__':main()
