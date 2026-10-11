"""Probe a broader fixed own-source contour with balanced false-positive cost."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse
import json
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
import restart64_butterfly07_source_ownership as ownership
from restart21_butterfly07_geometry_v2 import geometry,WING,HINGES


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--fit',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();args.output.mkdir(exist_ok=False);fit=json.loads(args.fit.read_text());scale=fit['fixed_uniform_scale'];basis=WING*scale
    def scaled_geometry(p):
        b,w=geometry(p);return b*scale,[a*scale for a in w]
    ownership.geometry=scaled_geometry;ownership.HINGES=np.array(HINGES)*scale
    rows=[ownership.prepare(r) for r in fit['rows']]
    def outline(values):
        result=basis.copy();result[1:6,:2]+=np.asarray(values).reshape(5,2);return result
    def objective(values):
        shape=outline(values);edge=np.roll(shape[:,:2],-1,axis=0)-shape[:,:2];cross=edge[:,0]*np.roll(edge[:,1],-1)-edge[:,1]*np.roll(edge[:,0],-1)
        if np.any(cross<0):return 1e9
        m=ownership.metrics(shape,rows)
        return m['missing']+2*m['bright_missing']+2*m['extra']+4*np.sum(np.asarray(values)**2)
    before=ownership.metrics(basis,rows)
    result=differential_evolution(objective,[(-1.6*scale,1.6*scale)]*10,x0=np.zeros(10),seed=110000+fit['animation_index'],popsize=5,maxiter=75,polish=False,tol=.002)
    values=result.x if objective(result.x)<objective(np.zeros(10)) else np.zeros(10)
    proposed=outline(values);after,per_phase=ownership.metrics(proposed,rows,True)
    np.savez_compressed(args.output/'anatomy.npz',wing_outline=proposed,parameters=np.asarray([r['parameters'] for r in fit['rows']]))
    report=dict(status='PRIVATE_OWN_SOURCE_ANATOMY_PROPOSAL',animation_index=fit['animation_index'],fit_sha256=ownership.sha(args.fit),recipe_sha256=ownership.sha(Path(__file__)),source_hashes_checked=99,before=before,after=after,wing_outline=proposed.tolist(),basis_outline=basis.tolist(),vertex_deltas=(proposed-basis).tolist(),phase_metrics=per_phase,pose_parameters_unchanged=True,objective='missing + 2*bright_missing + 2*extra + 4*contour_delta_squared',limits=['A single shared convex wing contour serves all99 phases; no per-frame scaling or background billboard.','Both hinge roots fixed. Five contour vertices may move at most1.6 scaled units per coordinate.','Body, head-tail direction and depth are still inferred; lower silhouette loss does not certify anatomy.','Regenerate source UV ownership and full continuous contact review before using this proposal.','No new approval, texture synthesis, canonical or runtime changes.'])
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(before=before,after=after)),flush=True)


if __name__=='__main__':main()
