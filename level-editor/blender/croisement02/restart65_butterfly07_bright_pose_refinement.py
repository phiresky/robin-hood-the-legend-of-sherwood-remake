"""Refine remaining bright silhouette mismatches with bounded rigid-pose proposals."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import copy,json
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as ref
BASE=ref.ROOT/'level-editor/work/active/croisement02'
FIT=BASE/'restart64-butterfly07-pose-refinement-v1/fit.json'
OUT=BASE/'restart65-butterfly07-bright-pose-refinement-v1'
def main():
 assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());original=fit['rows'];rows=copy.deepcopy(original);changes=[]
 for phase,row in enumerate(original):
  if row['bright_missing']<2:continue
  p=np.array(row['parameters']);ctx=ref.context(row['source']);before=original[(phase-1)%99];after=original[(phase+1)%99]
  bounds=[(v-20,v+20) for v in p[:5]]+[(v-.65,v+.65) for v in p[5:]]
  def loss(x):
   c=ref.evaluate(x,ctx)
   return (2*c['missing']+6*c['bright_missing']+c['extra'])/c['source_pixels']+.15*(ref.transition(before,c)+ref.transition(c,after))+.025*np.sum(((x-p)/np.array([20]*5+[.65]*2))**2)
  result=differential_evolution(loss,bounds,x0=p,seed=651100+phase,popsize=6,maxiter=95,polish=False,tol=.001)
  if loss(result.x)<loss(p):rows[phase].update(ref.evaluate(result.x,ctx));changes.append(phase)
  print(phase,row['bright_missing'],rows[phase]['bright_missing'],flush=True)
 edges=[]
 for phase,row in enumerate(rows):
  nxt=rows[(phase+1)%99];edges.append(dict(phase=phase,next_phase=(phase+1)%99,body_rotation_degrees=float(np.rad2deg((Rotation.from_quat(row['quaternion']).inv()*Rotation.from_quat(nxt['quaternion'])).magnitude())),hinge_delta_degrees=(np.array(nxt['parameters'][3:5])-row['parameters'][3:5]).tolist()))
 fit.update(status='PRIVATE_BRIGHT_POSE_REFINEMENT_PENDING_CONTACT_AND_RENDER',parent_sha256=ref.sha(FIT),recipe_sha256=ref.sha(Path(__file__)),rows=rows,edges=edges,missing_pixels=sum(r['missing'] for r in rows),bright_missing=sum(r['bright_missing'] for r in rows),extra_pixels=sum(r['extra'] for r in rows),second_pass_changes=changes,second_pass_bounds=dict(body_and_hinge_degrees=20,registration_pixels=.65),limits=fit['limits']+['Second-pass bright-mismatch poses allow20degree/.65pixel additional corrections. Their source-observed2D fit does not make hidden3D orientation observed.'])
 (OUT/'fit.json').write_text(json.dumps(fit,indent=2)+'\n');print({k:fit[k] for k in ('missing_pixels','bright_missing','extra_pixels')})
if __name__=='__main__':main()
