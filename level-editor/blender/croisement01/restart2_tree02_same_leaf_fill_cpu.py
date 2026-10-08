"""Propose bounded inferred RGB donors from the same physical leaf only."""
import hashlib,json
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';P=R/'tree02-leaf-donor-probe-v1';O=R/'tree02-same-leaf-fill-cpu-v1';O.mkdir(exist_ok=False)
d=json.loads((P/'mesh.json').read_text());a=np.load(P/'atlas.npz');own=a['ownership'];h,w=own.shape;rgba=np.stack([a['values_'+str(c)][a['index_'+str(c)]] for c in range(4)],axis=-1);tri={x['face']:x for x in d['triangles']};vertices={int(k):np.array(v) for k,v in d['vertices_world'].items()};rows=[];changes=[]
def samples(face):
 t=tri[face];uv=np.array(t['uv'])*[w,h];lo=np.maximum(0,np.floor(uv.min(0)-.5).astype(int));hi=np.minimum([w-1,h-1],np.ceil(uv.max(0)-.5).astype(int));yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];pixels=np.c_[xx.ravel(),yy.ravel()];bary=np.linalg.solve(np.vstack([uv.T,np.ones(3)]),np.vstack([(pixels+.5).T,np.ones(len(pixels))])).T;inside=(bary.min(1)>=1e-6);pixels=pixels[inside];points=bary[inside]@np.array([vertices[v] for v in t['vertices']]);return pixels,points
for leaf in d['leaves']:
 packets=[samples(f) for f in leaf];pixels=np.concatenate([x[0] for x in packets]);points=np.concatenate([x[1] for x in packets]);ownership=own[pixels[:,1],pixels[:,0]];generated=ownership==2;unknown=ownership==0;assert set(ownership)<={0,2};extent=float(np.linalg.norm(np.ptp(np.array([vertices[v] for f in leaf for v in tri[f]['vertices']]),axis=0)));row={'faces':leaf,'inside_texels':len(pixels),'generated_donors':int(generated.sum()),'unknown_targets':int(unknown.sum()),'leaf_diagonal':extent}
 if not generated.any():row['status']='NO_SAME_LEAF_DONOR';rows.append(row);continue
 donors=pixels[generated];dist,index=cKDTree(points[generated]).query(points[unknown]);targets=pixels[unknown];row['max_distance_fraction']=float(dist.max()/extent) if len(dist) else 0;row['status']='BOUNDED_SAME_LEAF_INFERENCE' if row['max_distance_fraction']<=.2 else 'DISTANCE_HOLD'
 for target,donor,distance in zip(targets,donors[index],dist):
  if distance/extent>.2:continue
  assert own[target[1],target[0]]==0 and own[donor[1],donor[0]]==2
  changes.append({'target':target.tolist(),'donor':donor.tolist(),'faces':leaf,'distance_fraction':float(distance/extent)})
 rows.append(row)
# These are explicit inferred RGB extensions, never native-owned pixels or blanket review-mask changes.
report={'model_sha256':d['model_sha256'],'probe_mesh_sha256':hashlib.sha256((P/'mesh.json').read_bytes()).hexdigest(),'probe_atlas_sha256':hashlib.sha256((P/'atlas.npz').read_bytes()).hexdigest(),'leaves':rows,'planned_texels':len(changes),'plan':changes,'alpha_unchanged':True,'geometry_uv_unchanged':True,'status':'CPU DONOR PLAN ONLY; SAVED MODEL AND VISUAL REVIEW PENDING','scope':'Only exact interior unknown texels of the selected physical leaves; nearest existing generated texel on the same connected leaf, maximum20% leaf diagonal. Generated donors remain inference. No cross-leaf donor, no mask changes.'};(O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(rows,indent=2));print('planned',len(changes))
