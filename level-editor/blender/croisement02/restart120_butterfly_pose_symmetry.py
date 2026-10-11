"""Resolve cyclic depth and paired-wing ambiguity without altering source silhouettes."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import argparse,copy,json,importlib.util
from pathlib import Path
import numpy as np
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as ref

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=False);fit=json.loads((a.candidate/'fit.json').read_text())
 spec=importlib.util.spec_from_file_location('own',a.candidate/'geometry.py');geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo);ref.geometry=geo.geometry
 flip=np.diag([1,1,-1]);turn=Rotation.from_euler('y',180,degrees=True).as_matrix();banks=[]
 for row in fit['rows']:
  params=np.array(row['parameters']);matrix=Rotation.from_euler('xyz',params[:3],degrees=True).as_matrix();ctx=ref.context(row['source']);bank=[]
  for mirror in [False,True]:
   for swap in [False,True]:
    r=matrix.copy();angles=params[3:5].copy()
    if mirror:r=flip@r@flip;angles=-angles
    if swap:r=r@turn;angles=-angles[::-1]
    q=params.copy();q[:3]=Rotation.from_matrix(r).as_euler('xyz',degrees=True);q[3:5]=angles
    value=ref.evaluate(q,ctx)
    assert (value['missing'],value['bright_missing'],value['extra'])==(row['missing'],row['bright_missing'],row['extra']),(row['phase'],mirror,swap)
    value.update(mirror_depth=mirror,swap_paired_wings=swap);bank.append(value)
  banks.append(bank)
 best=None
 for first in range(4):
  cost=np.full(4,np.inf);cost[first]=0;back=[]
  for i in range(1,99):
   grid=cost[:,None]+np.array([[ref.transition(x,y) for y in banks[i]] for x in banks[i-1]])
   indices=grid.argmin(0);cost=grid[indices,np.arange(4)];back.append(indices)
  cost+=np.array([ref.transition(x,banks[0][first]) for x in banks[-1]])
  last=int(cost.argmin());ids=[last]
  for indices in reversed(back):ids.append(int(indices[ids[-1]]))
  ids=list(reversed(ids))
  if best is None or cost[last]<best[0]:best=(float(cost[last]),ids)
 result=copy.deepcopy(fit)
 for row,bank,index in zip(result['rows'],banks,best[1]):row.update(bank[index])
 def turns(rows):return [float(np.degrees((Rotation.from_quat(r['quaternion']).inv()*Rotation.from_quat(rows[(i+1)%99]['quaternion'])).magnitude())) for i,r in enumerate(rows)]
 before=turns(fit['rows']);after=turns(result['rows']);result['status']='PRIVATE_SYMMETRY_RESOLVED_ARTICULATION_NOT_REVIEWED';result['symmetry_parent_sha256']=ref.sha(a.candidate/'fit.json');result['symmetry_recipe_sha256']=ref.sha(Path(__file__))
 (a.output/'fit.json').write_text(json.dumps(result,indent=2)+'\n')
 report=dict(status=result['status'],silhouette_metrics_identical_all99=True,body_turn_max_before=max(before),body_turn_max_after=max(after),body_turn_mean_before=float(np.mean(before)),body_turn_mean_after=float(np.mean(after)),changed_phases=sum(i!=0 for i in best[1]),branches=best[1],cyclic_transition_cost=best[0],limits=['Source projection cannot distinguish ray-depth mirror and paired wing labels. This chooses one temporally smoother physical hypothesis.','Hidden depth/surface identity and continuous interpolation changed, requiring fresh UV observations, contact and saved-model review.','No source artwork, native clock, centroid, rest shape or fixed scale changed.'])
 (a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
