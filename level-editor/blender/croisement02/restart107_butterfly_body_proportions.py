"""Fit one fixed body proportion to all own-source butterfly poses."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse,json
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
import restart64_butterfly07_source_ownership as ownership
from restart21_butterfly07_geometry_v2 import WING,HINGES,BODY,geometry


def main():
    ap=argparse.ArgumentParser();ap.add_argument('--fit',type=Path,required=True);ap.add_argument('--output',type=Path,required=True);args=ap.parse_args()
    args.output.mkdir(exist_ok=False);fit=json.loads(args.fit.read_text());scale=fit['fixed_uniform_scale']
    def geom(p):
        b,w=geometry(p);return b*scale,[a*scale for a in w]
    ownership.geometry=geom;ownership.HINGES=np.array(HINGES)*scale
    rows=[ownership.prepare(r) for r in fit['rows']]
    for c in rows:
        c['wings']=np.logical_or.reduce([Polygon((WING*scale)[:,:2]).contains_points(m['uv']) for m in c['maps']]);c['rot']=Rotation.from_euler('xyz',c['row']['parameters'][:3],degrees=True).as_matrix()
    def measure(x,detail=False):
        body=BODY.copy();body[:,0]*=x[0];body[:,1]*=x[1];body[:,1]+=x[2];body*=scale
        stats=[]
        for c in rows:
            xy=(body@c['rot'].T)[:,:2]+c['shift'];hit=Polygon(xy[ConvexHull(xy).vertices]).contains_points(c['q'])
            pred=c['wings']|hit;miss=c['target']&~pred
            stats.append(dict(phase=c['row']['phase'],missing=int(miss.sum()),bright_missing=int((miss&c['bright']).sum()),extra=int((pred&~c['target']).sum())))
        sums={k:sum(s[k] for s in stats) for k in ['missing','bright_missing','extra']}
        loss=sums['missing']+3*sums['bright_missing']+sums['extra']+5*np.sum((x-np.array([1,1,0]))**2)
        return (sums,stats) if detail else loss
    basis=np.array([1.,1.,0.]);before,_=measure(basis,True)
    result=differential_evolution(measure,[(.7,1.8),(1.,2.3),(-1.2,1.2)],x0=basis,seed=107000+fit['animation_index'],popsize=7,maxiter=45,tol=.001,polish=False)
    params=result.x if measure(result.x)<measure(basis) else basis
    after,phases=measure(params,True)
    report=dict(status='PRIVATE_FIXED_BODY_PROPORTION_PROBE',fit_sha256=ownership.sha(args.fit),recipe_sha256=ownership.sha(Path(__file__)),before=before,after=after,width_factor=float(params[0]),length_factor=float(params[1]),local_y_offset=float(params[2]),phase_metrics=phases,limits=['One fixed body mesh for all 99 poses, not per-frame scaling.','Wing silhouette, all body/wing rotations and source registration unchanged.','Body proportion and head-tail direction inferred from silhouette; no new source-color attribution yet.','Needs saved-model review and fresh contact/UV/export if adopted.'])
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report),flush=True)

if __name__=='__main__':main()
