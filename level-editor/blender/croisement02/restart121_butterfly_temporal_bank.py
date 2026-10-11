"""Compare cyclic source fidelity and physical turn costs on a fixed own anatomy."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import argparse,copy,json,importlib.util
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as ref

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads((a.candidate/'fit.json').read_text());spec=importlib.util.spec_from_file_location('geo',a.candidate/'geometry.py');geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo);ref.geometry=geo.geometry;banks=[]
 for i,row in enumerate(fit['rows']):
  ctx=ref.context(row['source']);bank=[]
  for offset in [0,-1,1,-2,2]:
   value=ref.evaluate(fit['rows'][(i+offset)%99]['parameters'],ctx);value['pose_seed_offset']=offset;bank.append(value)
  banks.append(bank)
 transitions=[]
 for i in range(99):transitions.append(np.array([[ref.transition(x,y) for y in banks[(i+1)%99]] for x in banks[i]]))
 summaries=[]
 for weight in [.08,.25,.5,1.]:
  best=None
  for first in range(5):
   cost=np.full(5,np.inf);cost[first]=banks[0][first]['loss'];back=[]
   for i in range(1,99):
    grid=cost[:,None]+weight*transitions[i-1];indices=grid.argmin(0);cost=grid[indices,np.arange(5)]+[v['loss'] for v in banks[i]];back.append(indices)
   cost+=weight*transitions[-1][:,first];last=int(cost.argmin());ids=[last]
   for indices in reversed(back):ids.append(int(indices[ids[-1]]))
   if best is None or cost[last]<best[0]:best=(float(cost[last]),list(reversed(ids)))
  result=copy.deepcopy(fit)
  for row,bank,index in zip(result['rows'],banks,best[1]):row.update(bank[index])
  turns=[float(np.degrees((Rotation.from_quat(r['quaternion']).inv()*Rotation.from_quat(result['rows'][(i+1)%99]['quaternion'])).magnitude())) for i,r in enumerate(result['rows'])]
  for key,field in [('missing_pixels','missing'),('bright_missing','bright_missing'),('extra_pixels','extra')]:result[key]=sum(r[field] for r in result['rows'])
  result['status']='PRIVATE_TEMPORAL_TRADEOFF_NOT_ADOPTED';result['temporal_parent_sha256']=ref.sha(a.candidate/'fit.json');result['temporal_weight']=weight
  name=f'weight-{weight:g}.json';(a.output/name).write_text(json.dumps(result,indent=2)+'\n');summary=dict(weight=weight,fit=name,missing=result['missing_pixels'],bright_missing=result['bright_missing'],extra=result['extra_pixels'],body_turn_max=max(turns),body_turn_mean=float(np.mean(turns)),changed_phases=sum(x!=0 for x in best[1]));summaries.append(summary);print(json.dumps(summary),flush=True)
 (a.output/'report.json').write_text(json.dumps(dict(status='PRIVATE_TRADEOFF_AUDIT_NOT_ACCEPTANCE',candidate_sha256=ref.sha(a.candidate/'fit.json'),recipe_sha256=ref.sha(Path(__file__)),variants=summaries,limits=['One fixed own-source anatomy; candidate poses borrowed from at most two adjacent frames, evaluated against each actual current frame.','Native99-frame duration and source centroids unchanged.','Any changed source silhouette and articulated pose needs own visual, UV, interpolation and contact review.']),indent=2)+'\n')
if __name__=='__main__':main()
