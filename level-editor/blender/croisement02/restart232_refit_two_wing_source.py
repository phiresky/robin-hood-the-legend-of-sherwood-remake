"""Refine selected two-wing source poses with neighboring physical constraints."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse,copy,json,importlib.util
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
from scipy.spatial import ConvexHull
from matplotlib.path import Path as Polygon
import restart21_butterfly07_global_registration_v2 as ref

def main():
 p=argparse.ArgumentParser();p.add_argument('--candidate',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--phases',type=int,nargs='+',required=True);p.add_argument('--passes',type=int,default=2);p.add_argument('--continuity-weight',type=float,default=.35);a=p.parse_args();a.output.mkdir(exist_ok=True)
 source=a.candidate/'fit.json';fit=json.loads(source.read_text());spec=importlib.util.spec_from_file_location('geo',a.candidate/'geometry.py');geo=importlib.util.module_from_spec(spec);spec.loader.exec_module(geo);ref.geometry=geo.geometry
 pins=dict(fit=ref.sha(source),geometry=ref.sha(a.candidate/'geometry.py'),recipe=ref.sha(Path(__file__)),passes=a.passes,continuity_weight=a.continuity_weight,phases=a.phases);pin=a.output/'pins.json'
 if pin.exists():assert json.loads(pin.read_text())==pins
 else:pin.write_text(json.dumps(pins,indent=2)+'\n')
 def evaluate(parameters,ctx):
  center,q,target,bright,dist=ctx;shift=center+np.asarray(parameters[5:7]);_,wings=geo.geometry(parameters);pred=np.zeros(len(q),bool)
  for body in [geo.geometry(parameters)[0]]:
   xy=body[:,:2]+shift;pred|=Polygon(xy[ConvexHull(xy).vertices]).contains_points(q)
  for wing in wings:pred|=Polygon(wing[:,:2]+shift).contains_points(q)
  missed=int((target&~pred).sum());extra=int((pred&~target).sum());bm=int((bright&~pred).sum());loss=(missed+2*bm+(1+.2*dist[pred&~target]).sum())/target.sum()+.012*float(np.sum(np.asarray(parameters[5:7])**2))
  return dict(parameters=list(map(float,parameters)),missing=missed,extra=extra,bright_missing=bm,covered=int((pred&target).sum()),source_pixels=int(target.sum()),loss=float(loss),quaternion=Rotation.from_euler('xyz',parameters[:3],degrees=True).as_quat().tolist())
 ref.evaluate=evaluate
 base_transition=ref.transition
 def transition(left,right):
  delta=np.array([])
  return base_transition(left,right)+.45*.30*float(delta@delta)
 ref.transition=transition
 contexts=[ref.context(r['source']) for r in fit['rows']]
 for row,ctx in zip(fit['rows'],contexts):row.update(evaluate(row['parameters'],ctx))
 state=a.output/'checkpoint.json'
 if state.exists():saved=json.loads(state.read_text());rows=saved['rows'];start=saved['next_step'];history=saved['history']
 else:rows=copy.deepcopy(fit['rows']);start=0;history=[]
 def energy():return sum(r['loss'] for r in rows)+a.continuity_weight*sum(ref.transition(r,rows[(i+1)%99]) for i,r in enumerate(rows))
 initial=energy();order=a.phases;assert len(set(order))==len(order) and all(0<=x<99 for x in order)
 for step in range(start,a.passes*len(order)):
  phase=order[step%len(order)];old=rows[phase];previous=rows[(phase-1)%99];following=rows[(phase+1)%99];base=Rotation.from_quat(old['quaternion']);prior=np.array(old['parameters']);ctx=contexts[phase]
  def unpack(x):
   q=prior.copy();q[:3]=(Rotation.from_rotvec(np.deg2rad(x[:3]))*base).as_euler('xyz',degrees=True);q[3:7]=x[3:7];return q
  def objective(x):
   r=ref.evaluate(unpack(x),ctx);return r['loss']+a.continuity_weight*(ref.transition(previous,r)+ref.transition(r,following))
  x0=np.r_[np.zeros(3),prior[3:7]];bounds=[(-25,25)]*3+[(-88,88),(-88,88),(-2.5,2.5),(-2.5,2.5)]
  result=differential_evolution(objective,bounds,x0=x0,seed=232000+step,popsize=7,maxiter=45,polish=False,tol=.001)
  chosen=result.x if objective(result.x)<objective(x0)-1e-9 else x0
  before=energy();updated=dict(old);updated.update(ref.evaluate(unpack(chosen),ctx));rows[phase]=updated;after=energy();assert after<=before+1e-8,(step,before,after)
  history.append(dict(step=step,phase=phase,joint_energy_before=before,joint_energy_after=after,missing_before=old['missing'],missing_after=updated['missing'],extra_before=old['extra'],extra_after=updated['extra']))
  tmp=state.with_suffix('.tmp');tmp.write_text(json.dumps(dict(next_step=step+1,rows=rows,history=history))+'\n');tmp.replace(state)
  print(step,phase,old['missing'],updated['missing'],round(before-after,6),flush=True)
 result=copy.deepcopy(fit);result.update(status='PRIVATE_JOINT_CYCLIC_SOURCE_FIT_NOT_REVIEWED',rows=rows,joint_fit_pins=pins)
 for key,field in [('missing_pixels','missing'),('bright_missing','bright_missing'),('extra_pixels','extra')]:result[key]=sum(r[field] for r in rows)
 turns=[float(np.degrees((Rotation.from_quat(r['quaternion']).inv()*Rotation.from_quat(rows[(i+1)%99]['quaternion'])).magnitude())) for i,r in enumerate(rows)]
 (a.output/'fit.json').write_text(json.dumps(result,indent=2)+'\n')
 report=dict(status=result['status'],joint_energy_final=energy(),updates=len(history),focused_phases=order,body_turn_max=max(turns),body_turn_mean=float(np.mean(turns)),missing=result['missing_pixels'],bright_missing=result['bright_missing'],extra=result['extra_pixels'],limits=['Every coordinate update minimizes source-frame coverage plus both current neighboring physical transitions, including98→0; no post-fit smoothing.','Local source-anatomy correction of undersized late-cycle broad lobes; no angular-speed limit; all other exact poses retained. Two inferred wing hinges, one fixed rest anatomy and scale. Continuity weight is an inferred regularizer, not native angular-speed evidence.','Source coverage changes are explicit and require visual review. New contact, texture ownership and actual imported-model proof remain mandatory.'])
 (a.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
