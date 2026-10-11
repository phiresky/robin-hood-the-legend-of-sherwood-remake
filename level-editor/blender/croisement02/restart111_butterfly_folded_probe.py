"""Inspect four-wing articulation in the smallest, folded source silhouettes."""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
os.environ.setdefault('OMP_NUM_THREADS','1')
import argparse,json,hashlib
from pathlib import Path
import numpy as np
from scipy.optimize import differential_evolution
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import restart64_butterfly07_source_ownership as ownership
from restart21_butterfly07_geometry_v2 import WING,HINGES,geometry


def split_y(poly,negative):
    out=[]
    for a,b in zip(poly,np.roll(poly,-1,axis=0)):
        ia=a[1]<=0 if negative else a[1]>=0
        ib=b[1]<=0 if negative else b[1]>=0
        if ia:out.append(a)
        if ia!=ib:out.append(a+(b-a)*(-a[1]/(b[1]-a[1])))
    return np.asarray(out)


def main():
    ap=argparse.ArgumentParser();ap.add_argument('--fit',type=Path,required=True);ap.add_argument('--output',type=Path,required=True);ap.add_argument('--count',type=int,default=16)
    args=ap.parse_args();args.output.mkdir(exist_ok=False);fit=json.loads(args.fit.read_text());scale=fit['fixed_uniform_scale']
    def geom(p):
        b,w=geometry(p);return b*scale,[x*scale for x in w]
    ownership.geometry=geom;ownership.HINGES=np.array(HINGES)*scale
    prepared=[ownership.prepare(r) for r in fit['rows']]
    outline=WING*scale;halves=[split_y(outline,True),split_y(outline,False)]
    selected=sorted(prepared,key=lambda c:int(c['target'].sum()))[:args.count]
    results=[]
    fig,axes=plt.subplots(4,4,figsize=(12,12),facecolor='#202020')
    for ax,c in zip(axes.flat,selected):
        p=np.array(c['row']['parameters']);rot=Rotation.from_euler('xyz',p[:3],degrees=True)
        def evaluate(x,detail=False):
            pred=c['body'].copy();polys=[]
            for side,sign in enumerate((-1,1)):
                for part,shape in enumerate(halves):
                    pts=shape.copy();pts[:,0]*=sign
                    pts=Rotation.from_euler('y',-sign*(p[3+side]+x[side*2+part]),degrees=True).apply(pts)
                    pts[:,0]+=HINGES[side]*scale
                    xy=rot.apply(pts)[:,:2]+c['shift'];polys.append(xy)
                    pred|=Polygon(xy).contains_points(c['q'])
            miss=c['target']&~pred
            stats=dict(missing=int(miss.sum()),bright_missing=int((miss&c['bright']).sum()),extra=int((pred&~c['target']).sum()))
            loss=stats['missing']+3*stats['bright_missing']+stats['extra']+.001*np.sum(x*x)
            return (stats,polys) if detail else loss
        base=evaluate(np.zeros(4),True)[0]
        result=differential_evolution(evaluate,[(-45,45)]*4,x0=np.zeros(4),seed=106000+c['row']['phase'],maxiter=55,popsize=7,polish=False,tol=.001)
        x=result.x if evaluate(result.x)<evaluate(np.zeros(4)) else np.zeros(4)
        after,polys=evaluate(x,True)
        row=dict(phase=c['row']['phase'],before=base,after=after,additional_hinges_degrees=x.tolist(),source=c['row']['source'])
        results.append(row);print(json.dumps(row),flush=True)
        ax.set_facecolor('#202020');ax.imshow(c['rgba'],interpolation='nearest')
        for poly,color in zip(polys,['cyan','dodgerblue','magenta','orange']):
            closed=np.vstack([poly,poly[0]])-.5;ax.plot(closed[:,0],closed[:,1],color=color,lw=.9)
        h,w=c['rgba'].shape[:2];ax.set_xlim(-3,w+3);ax.set_ylim(h+3,-3);ax.set_xticks([]);ax.set_yticks([])
        ax.set_title(f"{row['phase']}: miss {base['missing']}→{after['missing']}; extra {base['extra']}→{after['extra']}",color='white',fontsize=8)
    for ax in list(axes.flat)[len(selected):]:ax.axis('off')
    fig.tight_layout();fig.savefig(args.output/'worst-pose-source-overlay.png',dpi=140);plt.close(fig)
    report=dict(status='PRIVATE_ARTICULATION_PROBE_NOT_AN_APPROVED_MODEL',fit_sha256=ownership.sha(args.fit),recipe_sha256=ownership.sha(Path(__file__)),rows=results,rest_polygons=[h.tolist() for h in halves],before={k:sum(r['before'][k] for r in results) for k in results[0]['before']},after={k:sum(r['after'][k] for r in results) for k in results[0]['after']},limits=['Fixed body and pose registrations; unchanged total rest outline split at body midline into fore/hind wings.','Four independent extra hinge rotations limited to ±45 degrees. Split/root placement is an inferred diagnostic, not established anatomy.','Fits the smallest source silhouettes only, not temporally regularized; self-intersection, UV ownership and flight contact untested.','Do not adopt without full-cycle articulation, texture ownership, topology and context review.'])
    (args.output/'report.json').write_text(json.dumps(report,indent=2)+'\n')

if __name__=='__main__':main()
