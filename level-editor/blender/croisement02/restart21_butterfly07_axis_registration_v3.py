"""Bounded own-source axis refinement with periodic fixed-shape candidate selection."""
import copy,json,math
from concurrent.futures import ProcessPoolExecutor
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as fit

OUT=fit.B/'butterfly07-axis-registration-v3'
PARENT=fit.OUT/'fit.json'
LANDMARKS={18:([2.7,4.9],[-.65,.76]),19:([3.4,5.1],[.30,.95]),20:([4.5,2.8],[0.,1.]),21:([4.5,5.5],[.15,.99]),22:([2.8,5.8],[.30,.95]),91:([5.7,5.3],[.45,.89]),92:([2.8,6.1],[.30,.95]),93:([4.,6.9],[.73,.68])}

def evaluate(p,ctx,phase):
    c=fit.evaluate(p,ctx)
    if phase in LANDMARKS:
        landmark,axis=map(np.array,LANDMARKS[phase]);axis=axis/np.linalg.norm(axis)
        projected=Rotation.from_quat(c['quaternion']).apply([0.,1.,0.])[:2]
        projected/=max(np.linalg.norm(projected),1e-8)
        shift=ctx[0]+p[5:7]
        c['body_center_error_pixels']=float(np.linalg.norm(shift-landmark))
        c['body_axis_error_degrees']=math.degrees(math.acos(np.clip(projected@axis,-1,1)))
        c['loss']+=1.5*(float(np.sum((shift-landmark)**2))+4*float(np.sum((projected-axis)**2)))/c['source_pixels']
    return c

def axis_job(job):
    phase,source=job;ctx=fit.context(source);axis=np.array(LANDMARKS[phase][1]);zref=math.degrees(math.atan2(-axis[0],axis[1]));bank=[]
    bounds=[(-55,55),(-55,55),(zref-25,zref+25),(-88,88),(-88,88),(-2,2),(-2,2)]
    for seed in (8171,9181):
        result=differential_evolution(lambda p:evaluate(p,ctx,phase)['loss'],bounds,seed=seed+phase,popsize=6,maxiter=110,polish=False,tol=.003)
        for mirror in (False,True):
            p=result.x.copy()
            if mirror:p[[0,1,3,4]]*=-1
            c=evaluate(p,ctx,phase);c['axis_prior_candidate']=True;bank.append(c)
    return phase,bank

def select(banks):
    # Cache transition matrices once; the first-state enumeration closes the cycle.
    transitions=[np.array([[fit.transition(a,b) for b in banks[(phase+1)%99]] for a in banks[phase]]) for phase in range(99)]
    losses=[np.array([c['loss'] for c in banks[p]]) for p in range(99)];best=None
    for first in range(len(banks[0])):
        cost=np.full(len(banks[0]),np.inf);cost[first]=losses[0][first];back=[]
        for phase in range(98):
            total=cost[:,None]+transitions[phase];pi=total.argmin(0)
            cost=total[pi,np.arange(len(pi))]+losses[phase+1];back.append(pi)
        cost+=transitions[98][:,first];last=int(cost.argmin())
        if best is None or cost[last]<best[0]:
            ids=[last]
            for pi in reversed(back):ids.append(int(pi[ids[-1]]))
            best=(float(cost[last]),list(reversed(ids)))
    return [banks[p][i] for p,i in enumerate(best[1])]

def smooth_job(job):
    phase,source,previous,current,nextrow=job;ctx=fit.context(source)
    # Optimize fixed-size anatomy near the selected branch; neighbor penalties are
    # candidate proposals, not claimed biological angular-velocity measurements.
    p=np.array(current['parameters']);bounds=[(x-35,x+35) for x in p[:3]]+[(-88,88),(-88,88),(-2,2),(-2,2)]
    def loss(x):
        c=evaluate(x,ctx,phase)
        return c['loss']+fit.transition(previous,c)+fit.transition(c,nextrow)
    result=differential_evolution(loss,bounds,x0=p,seed=7613+phase,popsize=5,maxiter=55,polish=False,tol=.004)
    c=evaluate(result.x,ctx,phase);c['local_transition_refinement']=True
    return phase,c

def main():
    assert fit.sha(PARENT)=='b5525d6b7bd9aeebd41debbe63e4f3df0b4cf35159bfefcf33174fd9c27792cd'
    assert not OUT.exists();OUT.mkdir()
    parent=json.loads(PARENT.read_text());sources=[r['source'] for r in parent['rows']]
    banks={p:[evaluate(c['parameters'],fit.context(sources[p]),p) for c in json.loads((fit.OUT/f'phase-{p:03}.json').read_text())] for p in range(99)}
    with ProcessPoolExecutor(max_workers=2) as pool:
        for phase,bank in pool.map(axis_job,[(p,sources[p]) for p in LANDMARKS]):
            banks[phase].extend(bank);print('axis',phase,flush=True)
        original=copy.deepcopy(banks)
        for phase in range(99):
            ctx=fit.context(sources[phase])
            for neighbor in ((phase-1)%99,(phase+1)%99):
                banks[phase].extend(evaluate(c['parameters'],ctx,phase) for c in original[neighbor])
        selected=select(banks)
        jobs=[(p,sources[p],selected[(p-1)%99],selected[p],selected[(p+1)%99]) for p in range(99)]
        for phase,c in pool.map(smooth_job,jobs):
            banks[phase].append(c);print('refine',phase,flush=True)
    selected=select(banks);rows=[];edges=[]
    for phase,c in enumerate(selected):
        row=copy.deepcopy(c);row.update(phase=phase,source=sources[phase],locked_existing=False);rows.append(row)
        nxt=selected[(phase+1)%99]
        angle=math.degrees((Rotation.from_quat(c['quaternion']).inv()*Rotation.from_quat(nxt['quaternion'])).magnitude())
        edges.append(dict(phase=phase,next_phase=(phase+1)%99,body_rotation_degrees=angle,hinge_delta_degrees=(np.array(nxt['parameters'][3:5])-c['parameters'][3:5]).tolist()))
    out=copy.deepcopy(parent);out.update(parent_sha256=fit.sha(PARENT),recipe_sha256=fit.sha(fit.Path(__file__)),rows=rows,edges=edges,
        body_landmarks=LANDMARKS,body_landmark_uncertainty_pixels=1.5,body_axis_uncertainty_degrees=25,
        body_axis_uncertainty='Eight own-source warm-center and paired-wing-gap axes are uncertain hypotheses, not identified head/tail. Other phases have no manually inferred body axis.',
        missing_pixels=sum(r['missing'] for r in rows),extra_pixels=sum(r['extra'] for r in rows),bright_missing=sum(r['bright_missing'] for r in rows))
    (OUT/'fit.json').write_text(json.dumps(out,indent=2)+'\n')
    (OUT/'candidate-banks.json').write_text(json.dumps(banks)+'\n')
    print('DONE',out['missing_pixels'],out['extra_pixels'],out['bright_missing'],max(e['body_rotation_degrees'] for e in edges),flush=True)
if __name__=='__main__':main()
