"""Bounded source registration corrections without per-pose anatomy scaling."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import copy,json
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
import restart21_butterfly07_global_registration_v2 as ref
OUT=ref.ROOT/'level-editor/work/croisement02-refinement/restart64-butterfly07-pose-refinement-v1'
FIT=ref.B/'butterfly07-v3-baseline-contacts-v1/joined-fit.json'
def main():
    assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());original=fit['rows'];rows=[]
    for phase,row in enumerate(original):
        p=np.array(row['parameters']);ctx=ref.context(row['source']);before=original[(phase-1)%99];after=original[(phase+1)%99]
        bounds=[(v-8,v+8) for v in p[:5]]+[(v-.4,v+.4) for v in p[5:]]
        def loss(x):
            c=ref.evaluate(x,ctx)
            return (2*c['missing']+5*c['bright_missing']+c['extra'])/c['source_pixels']+.20*(ref.transition(before,c)+ref.transition(c,after))+.03*np.sum(((x-p)/np.array([8]*5+[.4]*2))**2)
        result=differential_evolution(loss,bounds,x0=p,seed=641100+phase,popsize=4,maxiter=40,polish=False,tol=.001)
        best=result.x if loss(result.x)<loss(p) else p
        changed=copy.deepcopy(row);changed.update(ref.evaluate(best,ctx));rows.append(changed)
        print(phase,row['missing'],changed['missing'],row['bright_missing'],changed['bright_missing'],flush=True)
    edges=[]
    for phase,row in enumerate(rows):
        nxt=rows[(phase+1)%99];edges.append(dict(phase=phase,next_phase=(phase+1)%99,body_rotation_degrees=float(np.rad2deg((Rotation.from_quat(row['quaternion']).inv()*Rotation.from_quat(nxt['quaternion'])).magnitude())),hinge_delta_degrees=(np.array(nxt['parameters'][3:5])-row['parameters'][3:5]).tolist()))
    fit.update(status='PRIVATE_BOUNDED_REGISTRATION_PENDING_RENDER_CONTACT_REVIEW',parent_sha256=ref.sha(FIT),recipe_sha256=ref.sha(Path(__file__)),rows=rows,edges=edges,missing_pixels=sum(r['missing'] for r in rows),bright_missing=sum(r['bright_missing'] for r in rows),extra_pixels=sum(r['extra'] for r in rows),refinement_bounds=dict(body_and_hinge_degrees=8,registration_pixels=.4),limits=['Fixed wing/body anatomy unchanged across every phase.','Body/wing axes are inferred; bounded local corrections are not observed 3D motion.','Exact source frame timing, bbox and display flight path retained; camera ray depth unchanged.','Existing collision proof does not certify these corrected poses.','No synthesized texture; source pattern reconstruction remains separate.'])
    (OUT/'fit.json').write_text(json.dumps(fit,indent=2)+'\n');print({k:fit[k] for k in ('missing_pixels','bright_missing','extra_pixels')})
if __name__=='__main__':main()
