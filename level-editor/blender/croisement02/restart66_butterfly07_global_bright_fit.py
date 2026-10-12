"""Test global rigid-pose alternatives for remaining bright source mismatches."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,copy
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as ref
BASE=ref.ROOT/'level-editor/work/active/croisement02';FIT=BASE/'restart65-butterfly07-bright-pose-refinement-v1/fit.json';OUT=BASE/'restart66-butterfly07-global-bright-fit-v1'
def main():
 assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());original=fit['rows'];rows=copy.deepcopy(original)
 for phase,row in enumerate(original):
  if not row['bright_missing']:continue
  ctx=ref.context(row['source']);before=original[(phase-1)%99];after=original[(phase+1)%99]
  def loss(x):
   c=ref.evaluate(x,ctx);return (2*c['missing']+8*c['bright_missing']+c['extra'])/c['source_pixels']+.07*(ref.transition(before,c)+ref.transition(c,after))
  p=np.array(row['parameters']);bounds=[(v-55,v+55) for v in p[:3]]+[(-88,88),(-88,88),(-2.5,2.5),(-2.5,2.5)]
  result=differential_evolution(loss,bounds,x0=np.clip(p,np.array(bounds)[:,0],np.array(bounds)[:,1]),seed=661100+phase,popsize=6,maxiter=130,polish=False,tol=.001)
  if loss(result.x)<loss(p):rows[phase].update(ref.evaluate(result.x,ctx))
  print(phase,row['bright_missing'],rows[phase]['bright_missing'],row['missing'],rows[phase]['missing'],flush=True)
 edges=[]
 for phase,row in enumerate(rows):
  nxt=rows[(phase+1)%99];edges.append(dict(phase=phase,next_phase=(phase+1)%99,body_rotation_degrees=float(np.rad2deg((Rotation.from_quat(row['quaternion']).inv()*Rotation.from_quat(nxt['quaternion'])).magnitude())),hinge_delta_degrees=(np.array(nxt['parameters'][3:5])-row['parameters'][3:5]).tolist()))
 fit.update(status='PRIVATE_GLOBAL_RIGID_POSE_ALTERNATIVES',parent_sha256=ref.sha(FIT),recipe_sha256=ref.sha(Path(__file__)),rows=rows,edges=edges,missing_pixels=sum(r['missing'] for r in rows),bright_missing=sum(r['bright_missing'] for r in rows),extra_pixels=sum(r['extra'] for r in rows),limits=['Global rigid pose alternatives remain unapproved and require renewed motion/receiver review.','Fixed geometry, exact source frames and timing unchanged.','No topology or per-frame scale changes.','Depth branches remain inferred; a better silhouette does not establish true attitude.'])
 (OUT/'fit.json').write_text(json.dumps(fit,indent=2)+'\n');print({k:fit[k] for k in ('missing_pixels','bright_missing','extra_pixels')})
if __name__=='__main__':main()
