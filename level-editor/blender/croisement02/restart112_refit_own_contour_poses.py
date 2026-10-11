"""Alternate own-source fixed anatomy with source-pose fitting, checkpointed by phase."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse,json
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as ref
from restart21_butterfly07_geometry_v2 import BODY,HINGES

def main():
 p=argparse.ArgumentParser();p.add_argument('--fit',type=Path,required=True);p.add_argument('--anatomy',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();a.output.mkdir(exist_ok=True)
 fit=json.loads(a.fit.read_text());anatomy=json.loads(a.anatomy.read_text());assert anatomy['fit_sha256']==ref.sha(a.fit)
 pins=dict(fit=ref.sha(a.fit),anatomy=ref.sha(a.anatomy),recipe=ref.sha(Path(__file__)));pin=a.output/'pins.json'
 if pin.exists():assert json.loads(pin.read_text())==pins
 else:pin.write_text(json.dumps(pins,indent=2)+'\n')
 scale=fit['fixed_uniform_scale'];wing=np.array(anatomy['wing_outline'])
 def geometry(params):
  rotation=Rotation.from_euler('xyz',params[:3],degrees=True);wings=[]
  for side,sign in enumerate((-1,1)):
   pts=wing.copy();pts[:,0]*=sign;pts=Rotation.from_euler('y',-sign*params[3+side],degrees=True).apply(pts);pts[:,0]+=HINGES[side]*scale;wings.append(rotation.apply(pts))
  return rotation.apply(BODY*scale),wings
 ref.geometry=geometry;rows=[]
 for phase,old in enumerate(fit['rows']):
  ctx=ref.context(old['source']);prior=np.array(old['parameters']);dest=a.output/f'phase-{phase:03}.json'
  if dest.exists():
   row=json.loads(dest.read_text());assert row['source']==old['source']
   assert ref.evaluate(row['parameters'],ctx)['missing']==row['missing'];rows.append(row);continue
  before=fit['rows'][(phase-1)%99];after=fit['rows'][(phase+1)%99]
  def objective(parameters):
   r=ref.evaluate(parameters,ctx);return r['loss']+.08*(ref.transition(before,r)+ref.transition(r,after))
  bounds=[(x-35,x+35) for x in prior[:3]]+[(-88,88),(-88,88),(-2.5,2.5),(-2.5,2.5)]
  result=differential_evolution(objective,bounds,x0=np.clip(prior,np.array(bounds)[:,0],np.array(bounds)[:,1]),seed=112000+phase,maxiter=60,popsize=6,polish=False,tol=.001)
  chosen=result.x if objective(result.x)<objective(prior) else prior
  row=dict(old);row.update(ref.evaluate(chosen,ctx));rows.append(row);dest.write_text(json.dumps(row)+'\n');print(phase,old['missing'],row['missing'],old['extra'],row['extra'],flush=True)
 report=dict(fit,status='PRIVATE_OWN_ANATOMY_AND_POSE_PROPOSAL_NOT_REVIEWED',rows=rows,source_pins=pins,fixed_wing_outline_scaled=wing.tolist())
 for key,field in [('missing_pixels','missing'),('bright_missing','bright_missing'),('extra_pixels','extra'),('source_pixels','source_pixels')]:report[key]=sum(r[field] for r in rows)
 report['limitations']=fit['limitations']+['Own-source outline differs from prior helper; consumers must explicitly use fixed_wing_outline_scaled.','New articulation and UV ownership, body landmarks, interpolation contacts and saved-model review remain pending.']
 (a.output/'fit.json').write_text(json.dumps(report,indent=2)+'\n');print({k:report[k] for k in ['missing_pixels','bright_missing','extra_pixels','source_pixels']},flush=True)
if __name__=='__main__':main()
