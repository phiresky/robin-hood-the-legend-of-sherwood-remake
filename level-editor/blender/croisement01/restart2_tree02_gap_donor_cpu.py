"""Diagnose exact same-face donors without crossing protected review masks."""
import json,collections
from pathlib import Path
import numpy as np
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';P=R/'tree02-rendered-gap-probe-v1';d=json.loads((P/'report.json').read_text());targets=collections.defaultdict(set)
for witness in d['witnesses']:
 for hit in witness['hits']:
  if hit['ownership']==0:targets[(hit['object'],hit['face'])].add(tuple(hit['atlas_texel']))
rows=[]
for (name,face),texels in targets.items():
 p=d['packets'][name];verts={int(k):np.array(v) for k,v in p['vertices_world'].items()};triangles=[t for t in p['triangles'] if t['face']==face];samples={}
 for t in triangles:
  uv=np.array(t['uv'])*p['atlas_size'];points=np.array([verts[i] for i in t['vertices']])
  for key,val in p['texels'].items():
   xy=np.array(list(map(int,key.split(','))));weights=np.linalg.solve(np.vstack([uv.T,np.ones(3)]),np.r_[xy+.5,1])
   if weights.min()>=-1e-6:samples[tuple(map(int,xy))]=(val,weights@points)
 donors={xy:v for xy,v in samples.items() if v[0]['ownership'] in [1,2]};plans=[]
 for xy in texels:
  if xy not in samples or not donors:continue
  point=samples[xy][1];donor=min(donors,key=lambda q:np.linalg.norm(donors[q][1]-point));distance=float(np.linalg.norm(donors[donor][1]-point));plans.append({'target':list(xy),'donor':list(donor),'donor_ownership':donors[donor][0]['ownership'],'distance_world':distance,'distance_atlas':float(np.linalg.norm(np.array(donor)-xy))})
 rows.append({'object':name,'face':face,'targets':[list(x) for x in texels],'valid_same_face_donor_count':len(donors),'supported_interior_target_plans':plans,'requires_more_evidence':len(plans)!=len(texels)})
report={'model_sha256':d['model_sha256'],'rows':rows,'status':'DIAGNOSIS ONLY; NO REPAIR APPLIED','scope':'Test both loop triangles of each physical polygon; do not misclassify a quad using only its first triangle. Source donors, if used later, must create inferred provenance rather than expanding source ownership.'};(P/'same-face-donor-diagnosis.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([x for x in rows if x['supported_interior_target_plans']],indent=2))
