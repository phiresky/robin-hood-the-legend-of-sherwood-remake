"""Replace flat-topped rock relief with full convex source-outlined mass profiles."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
import restart2_bank_rounded_back_cpu as back
from restart2_bank_boulder_faces import inside,project
B=back.B;O=B/'bank-convex-masses-cpu-v1';E=B/'bank-convex-east-cpu-v1';RAY=np.array([0.,-back.C,back.S])
def distance(p,poly):
 result=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  a=np.array(a);v=np.array(b)-a;t=np.clip(np.dot(p-a,v)/np.dot(v,v),0,1);result.append(np.linalg.norm(p-a-t*v))
 return min(result)
def faces(mesh,owner,proposals):
 d=dict(mesh);v=np.array(mesh['vertices']);old=v.copy();front=mesh['faces'][-mesh['front_reconstruction']['new_front_triangles']:];ids=set(i for f in front for i in f);edges=Counter(tuple(sorted((a,b)))for f in front for a,b in zip(f,f[1:]+f[:1]));boundary={i for edge,n in edges.items()if n==1 for i in edge};xy=project(v);oldrelief=np.zeros(len(v));newrelief=np.zeros(len(v));details=[]
 for face in proposals:
  poly=face['native_outline'];samples={i:distance(xy[i],poly)for i in ids if inside(xy[i],poly)};radius=max(samples.values(),default=1);span=min(np.ptp(poly,axis=0));amplitude=min(14.,span*.72)
  for i,dist in samples.items():
   oldrelief[i]=max(oldrelief[i],min(12.,span*.70)*math.sin(min(1.,dist/max(2.,span*.30))*math.pi/2));fraction=np.clip(dist/radius,0,1);newrelief[i]=max(newrelief[i],amplitude*math.sqrt(max(0.,fraction*(2-fraction))))
  details.append({'id':face['id'],'interior_radius':float(radius),'amplitude':float(amplitude),'samples':len(samples)})
 for i in ids-boundary:
  change=newrelief[i]-oldrelief[i]
  if owner==54 and 286<xy[i,1]<350:
   # Broad upper shoulder is partly foliage-covered. Its enclosing curvature
   # is an explicit hypothesis, bounded by the retained front silhouette.
   poly=mesh['front_reconstruction']['native_boundary'];edge=distance(xy[i],poly);profile=math.sin(math.pi*(xy[i,1]-286)/64);change+=min(18.,edge*1.3)*max(0.,profile)
  v[i]+=RAY*change
 assert np.max(np.abs(project(v)-xy))<1e-10;assert np.array_equal(v[list(boundary)],old[list(boundary)])
 d['vertices']=v.tolist();d['convex_mass_revision']={'old_world_sha256':hashlib.sha256(old.tobytes()).hexdigest(),'source_2d_error':float(np.max(np.abs(project(v)-xy))),'boundary_vertices_exact':len(boundary),'faces':details,'changes':[{'vertex':i,'old_world':old[i].tolist(),'new_world':v[i].tolist()}for i in sorted(ids)if not np.array_equal(old[i],v[i])],'limits':['Depth is inferred; all observed2D traces and perimeter remain exact.','Old flat central displacement plateaus removed. Upper covered shoulder convexity is hypothesis, not source ownership.']};return d

def main():
 assert not O.exists()and not E.exists();O.mkdir();E.mkdir();d=json.loads((B/'bank-boulder-fronts-cpu-v8/geometry.json').read_text());e=json.loads((B/'bank-boulder-east-cpu-v8/geometry.json').read_text());proposals=json.loads((B/'bank-visible-face-traces-v1/faces.json').read_text())['faces'];d['54']=faces(d['54'],54,[p for p in proposals if p['owner']==54]);e=faces(e,52,[p for p in proposals if p['owner']==52]);template=json.loads((B/'bank-shoulder-lofts-cpu-v7/geometry.json').read_text())['54'];et=json.loads((B/'bank-east-loft-cpu-v8/geometry.json').read_text());d['54']=back.rebuild(d['54'],template,54);e=back.rebuild(e,et,52)
 for t in d['52']['traces']:
  if not t['id'].startswith('east-'):continue
  trace=next(r for r in e['traces']if r['id']==t['id']);t['source_path_world']=[e['vertices'][i]for i in trace['vertices']]
 (O/'geometry.json').write_text(json.dumps(d,separators=(',',':'))+'\n');(E/'geometry.json').write_text(json.dumps(e,separators=(',',':'))+'\n');print({'south_changed_front':len(d['54']['convex_mass_revision']['changes']),'east_changed_front':len(e['convex_mass_revision']['changes'])})
if __name__=='__main__':main()
