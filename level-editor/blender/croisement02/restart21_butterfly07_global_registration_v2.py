"""Refit all99 own butterfly07 poses using one globally revised restshape."""
import copy, hashlib, json, math, os
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
from restart21_butterfly07_geometry_v2 import geometry, fixed_geometry
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'
OUT=B/'butterfly07-global-registration-v2'
PARENT=B/'butterfly07-full-local-registration-v1/fit.json'
PLAN=B/'all7-context-plan-v1/plan.json'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def context(source):
    assert sha(Path(source['source']))==source['sha256']
    a=np.asarray(Image.open(source['source']).convert('RGBA'));mask=a[:,:,3]>0;h,w=mask.shape
    yy,xx=np.nonzero(mask);center=np.array([xx.mean()+.5,yy.mean()+.5])
    gy,gx=np.mgrid[-3:h+3,-3:w+3];q=np.c_[gx.ravel()+.5,gy.ravel()+.5]
    target=np.zeros(gx.shape,bool);target[3:h+3,3:w+3]=mask
    bright=np.zeros(gx.shape,bool);bright[3:h+3,3:w+3]=mask&(a[:,:,:3].max(2)>=100)
    return center,q,target.ravel(),bright.ravel(),distance_transform_edt(~target).ravel()
def evaluate(p,ctx):
    center,q,target,bright,dist=ctx;body,wings=geometry(p);shift=center+np.asarray(p[5:7])
    b=body[:,:2]+shift;pred=Polygon(b[ConvexHull(b).vertices]).contains_points(q)
    for w in wings:pred|=Polygon(w[:,:2]+shift).contains_points(q)
    missed=int((target&~pred).sum());extra=int((pred&~target).sum());bm=int((bright&~pred).sum())
    loss=(missed+2*bm+(1+.2*dist[pred&~target]).sum())/target.sum()+.012*float(np.sum(np.asarray(p[5:7])**2))
    return dict(parameters=list(map(float,p)),missing=missed,extra=extra,bright_missing=bm,
        covered=int((pred&target).sum()),source_pixels=int(target.sum()),loss=float(loss),
        quaternion=Rotation.from_euler('xyz',p[:3],degrees=True).as_quat().tolist())
def fit_job(job):
    phase,source,prior=job;ctx=context(source);bank=[]
    bounds=[(-85,85),(-85,85),(-180,180),(-88,88),(-88,88),(-2,2),(-2,2)]
    for seed in (21711,49117):
        result=differential_evolution(lambda p:evaluate(p,ctx)['loss'],bounds,seed=seed+phase,
            popsize=5,maxiter=90,polish=False,tol=.003,x0=np.clip(prior,np.array(bounds)[:,0],np.array(bounds)[:,1]))
        for mirrored in (False,True):
            p=result.x.copy()
            if mirrored:p[[0,1,3,4]]*=-1
            c=evaluate(p,ctx);c.update(seed=seed,depth_mirrored=mirrored);bank.append(c)
    bank.append(evaluate(prior,ctx))
    return phase,bank
def transition(a,b):
    angle=(Rotation.from_quat(a['quaternion']).inv()*Rotation.from_quat(b['quaternion'])).magnitude()
    hinge=np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5])
    registration=np.array(a['parameters'][5:7])-b['parameters'][5:7]
    return .45*(angle**2+.30*float(hinge@hinge))+.025*float(registration@registration)
def main():
    assert sha(PARENT)=='2692955a7812eba59f0d38e20044915286db8309a167955ee4fc45c399d4ded5'
    assert not OUT.exists();OUT.mkdir()
    parent=json.loads(PARENT.read_text());seq=next(s for s in json.loads(PLAN.read_text())['sequences'] if s['index']==14)
    prior_rows={r['phase']:r for r in parent['rows']};fixed={};assert len(prior_rows)==99 and len(seq['path'])==99
    banks={p:[evaluate(r['parameters'],context(seq['path'][p]))] for p,r in fixed.items()}
    jobs=[]
    for phase in range(99):
        if phase in fixed:continue
        jobs.append((phase,seq['path'][phase],prior_rows[phase]['parameters']))
    with ProcessPoolExecutor(max_workers=2) as pool:
        for phase,bank in pool.map(fit_job,jobs):
            banks[phase]=bank
            (OUT/f'phase-{phase:03}.json').write_text(json.dumps(bank)+'\n')
            print('phase',phase,'best loss',round(min(c['loss'] for c in bank),4),flush=True)
    for phase in range(99):
        ctx=context(seq['path'][phase]);extra=[]
        for neighbor in [(phase-1)%99,(phase+1)%99]:
            for candidate in banks[neighbor][:4]:extra.append(evaluate(candidate['parameters'],ctx))
        banks[phase].extend(extra)
    # Enumerate all phase0 states to close the true99->0 seam, no locked anatomy poses.
    best=None
    for first in range(len(banks[0])):
        cost=np.full(len(banks[0]),float('inf'));cost[first]=banks[0][first]['loss'];back=[]
        for prev,current in zip(range(98),range(1,99)):
            total=cost[:,None]+np.array([[transition(a,b) for b in banks[current]] for a in banks[prev]])
            pi=total.argmin(0);cost=total[pi,np.arange(len(pi))]+[c['loss'] for c in banks[current]];back.append(pi)
        cost+=np.array([transition(c,banks[0][first]) for c in banks[98]])
        last=int(cost.argmin())
        if best is None or cost[last]<best[0]:
            ids=[last]
            for pi in reversed(back):ids.append(int(pi[ids[-1]]))
            best=(float(cost[last]),list(reversed(ids)))
    selected={p:banks[p][i] for p,i in enumerate(best[1])};rows=[];edges=[]
    for phase in range(99):
        row=copy.deepcopy(selected[phase]);row.update(phase=phase,source=seq['path'][phase],locked_existing=phase in fixed)
        if phase in fixed:assert row['parameters']==fixed[phase]['parameters']
        rows.append(row);nxt=selected[(phase+1)%99]
        angle=math.degrees((Rotation.from_quat(row['quaternion']).inv()*Rotation.from_quat(nxt['quaternion'])).magnitude())
        edges.append(dict(phase=phase,next_phase=(phase+1)%99,body_rotation_degrees=angle,
            hinge_delta_degrees=(np.array(nxt['parameters'][3:5])-row['parameters'][3:5]).tolist()))
    out=dict(status='LOCAL_99_POSE_HYPOTHESES_NOT_RENDER_OR_CLEARANCE_PROOF',parent_sha256=sha(PARENT),plan_sha256=sha(PLAN),
        recipe_sha256=sha(Path(__file__)),own_source_only=True,fixed_geometry=fixed_geometry(),geometry_helper_sha256=sha(Path(__file__).with_name('restart21_butterfly07_geometry_v2.py')),fixed_material_proposal=parent['fixed_material_proposal'],
        rows=rows,edges=edges,locked_phases=sorted(fixed),body_axis_uncertainty='No new body landmarks are claimed; silhouette permits mirrored ray depth and ambiguous head/tail.',
        missing_pixels=sum(r['missing'] for r in rows),extra_pixels=sum(r['extra'] for r in rows),source_pixels=sum(r['source_pixels'] for r in rows),
        bright_missing=sum(r['bright_missing'] for r in rows),limits=['No depth/path edits; local camera coordinates only.','Native99bbox/centroid/timing retained exactly.','Dark filaments and other missing source pixels explicitly remain unmodeled.','Fixed palette unchanged; no detailed UV pattern or physical light fit.','Discrete phase fits and regularized transitions do not establish continuous anatomical or collision validity.'])
    (OUT/'fit.json').write_text(json.dumps(out,indent=2)+'\n');print('DONE',out['missing_pixels'],out['extra_pixels'],out['source_pixels'],flush=True)
if __name__=='__main__':main()
