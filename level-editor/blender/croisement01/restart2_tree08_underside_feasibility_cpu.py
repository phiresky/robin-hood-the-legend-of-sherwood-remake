"""Measure source-projection-preserving underside extensions; no mesh mutation."""
import json
import numpy as np
from restart2_tree08_root_embedding_cpu import R,P,mesh
O=R/'tree08-underside-feasibility-cpu-v1';O.mkdir(exist_ok=False)
receipt=json.loads((P/'current-contact/receipt.json').read_text());audit=json.loads((R/'tree08-root-embedding-cpu-v1/report.json').read_text());direction=np.array([0.,np.cos(np.radians(35)),-np.sin(np.radians(35))]);receivers=[]
for binding in receipt['bindings']:
 v,f=mesh(binding['asset']);v+=binding['translation'];receivers.append((binding['asset']['id'],v[f]))
def first_hit(origin,t):
 e1=t[:,1]-t[:,0];e2=t[:,2]-t[:,0];h=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,h);ok=abs(det)>1e-10;inv=np.where(ok,1/np.where(ok,det,1),0);s=origin-t[:,0];u=inv*np.einsum('ij,ij->i',s,h);q=np.cross(s,e1);v=inv*(q@direction);distance=inv*np.einsum('ij,ij->i',e2,q);ok&=(u>=-1e-8)&(v>=-1e-8)&(u+v<=1+1e-8)&(distance>=0)
 return float(distance[ok].min()) if ok.any() else None
rows=[]
for a in audit['samples']:
 if a['status']!='FLOATING':continue
 origin=np.array([*a['wood_hit'][:2],a['matched_intervals'][0][0]])
 candidates=[(name,first_hit(origin,t)) for name,t in receivers];candidates=[(name,d) for name,d in candidates if d is not None]
 assert candidates
 name,d=min(candidates,key=lambda x:x[1]);target=origin+direction*d
 rows.append({'native':a['native'],'route':a['route'],'underside':origin.tolist(),'target_receiver':name,'source_ray_extension':d,'target':target.tolist(),'original_vertical_gap':a['bottom_minus_receiver']})
out={'model_sha256':receipt['model_sha256'],'samples':rows,'max_ray_extension':max(x['source_ray_extension'] for x in rows),'receiver_counts':{name:sum(x['target_receiver']==name for x in rows) for name in set(x['target_receiver'] for x in rows)},'status':'FEASIBILITY ONLY; NO NEW MESH','construction_constraints':['Move inferred underside backward along native sight rays, leaving source-facing first-hit surfaces fixed.','A smooth thickness field must be solved over connected root cross-sections; independently shifting triangle vertices is not a validated mesh operation.','Check native first-hit depth and silhouette, closed orientation and self-intersections before model construction.','Receiver intersections along the backward ray may reach the terrace side rather than vertically downward ground. Review natural taper and full contact in oblique views.','Upper protected core remains exact; source roots need independent known-pixel ownership guard.']}
(O/'report.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps({k:v for k,v in out.items() if k not in ['samples','construction_constraints']}))
