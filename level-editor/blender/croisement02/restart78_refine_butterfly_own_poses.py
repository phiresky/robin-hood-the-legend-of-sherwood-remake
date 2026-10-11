"""Refine a private butterfly pose sequence against its own source artwork."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse
import json
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
import restart21_butterfly07_global_registration_v2 as ref
from restart21_butterfly07_geometry_v2 import geometry


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--input',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    args.output.mkdir(exist_ok=False)
    seed=json.loads(args.input.read_text())
    scale=seed['fixed_uniform_scale']
    def scaled_geometry(p):
        body,wings=geometry(p)
        return body*scale,[w*scale for w in wings]
    ref.geometry=scaled_geometry
    rows=[]
    for phase,old in enumerate(seed['rows']):
        ctx=ref.context(old['source'])
        before=seed['rows'][(phase-1)%99]
        after=seed['rows'][(phase+1)%99]
        p=np.array(old['parameters'])
        bounds=[(v-55,v+55) for v in p[:3]]+[(-88,88),(-88,88),(-2.5,2.5),(-2.5,2.5)]
        def objective(parameters):
            row=ref.evaluate(parameters,ctx)
            return row['loss']+.06*(ref.transition(before,row)+ref.transition(row,after))
        result=differential_evolution(objective,bounds,x0=np.clip(p,np.array(bounds)[:,0],np.array(bounds)[:,1]),seed=780000+seed['animation_index']*100+phase,popsize=5,maxiter=35,polish=False,tol=.002)
        row=dict(old)
        if objective(result.x)<objective(p):
            row.update(ref.evaluate(result.x,ctx))
        rows.append(row)
        (args.output/f'phase-{phase:03}.json').write_text(json.dumps(row)+'\n')
        print(phase,old['missing'],row['missing'],old['bright_missing'],row['bright_missing'],flush=True)
    report=dict(seed,status='PRIVATE_OWN_SOURCE_POSE_REFINEMENT_NOT_REVIEW_READY',parent_sha256=ref.sha(args.input),recipe_sha256=ref.sha(Path(__file__)),rows=rows)
    for key,field in [('missing_pixels','missing'),('bright_missing','bright_missing'),('extra_pixels','extra'),('source_pixels','source_pixels')]:
        report[key]=sum(r[field] for r in rows)
    report['limitations']=seed['limitations']+['Own-art pose optimization completed; topology/texture ownership, temporal smoothness, full path and contact review remain.']
    (args.output/'fit.json').write_text(json.dumps(report,indent=2)+'\n')
    print({k:report[k] for k in ('missing_pixels','bright_missing','extra_pixels','source_pixels')},flush=True)


if __name__=='__main__':
    main()
