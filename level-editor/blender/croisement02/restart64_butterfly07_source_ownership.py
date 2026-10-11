"""Bounded fixed-anatomy refinement and observed-only wing texture evidence."""
import hashlib,json,os
from pathlib import Path
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import numpy as np
from PIL import Image
from scipy.optimize import differential_evolution
from scipy.spatial import ConvexHull
from scipy.spatial.transform import Rotation
from matplotlib.path import Path as Polygon
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from restart21_butterfly07_geometry_v2 import WING,geometry,HINGES
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'
FIT=BASE/'butterfly07-v3-baseline-contacts-v1/joined-fit.json'
OUT=ROOT/'level-editor/work/croisement02-refinement/restart64-butterfly07-source-ownership-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()

def prepare(row):
    p=np.array(row['parameters']);source=Path(row['source']['source']);assert sha(source)==row['source']['sha256']
    rgba=np.array(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];mask=rgba[:,:,3]>0
    yy,xx=np.nonzero(mask);shift=np.array([xx.mean()+.5,yy.mean()+.5])+p[5:7]
    gy,gx=np.mgrid[-3:h+3,-3:w+3];q=np.c_[gx.ravel()+.5,gy.ravel()+.5]
    target=np.zeros(gx.shape,bool);target[3:h+3,3:w+3]=mask
    bright=np.zeros_like(target);bright[3:h+3,3:w+3]=mask&(rgba[:,:,:3].max(2)>=100)
    body,_=geometry(p);b=body[:,:2]+shift;bodyhit=Polygon(b[ConvexHull(b).vertices]).contains_points(q)
    rot=Rotation.from_euler('xyz',p[:3],degrees=True).as_matrix();maps=[]
    for side,sign in enumerate((-1,1)):
        turn=Rotation.from_euler('y',-sign*p[3+side],degrees=True).as_matrix()
        matrix=rot@turn@np.diag([sign,1,1]);offset=rot@np.array([HINGES[side],0,0])
        uv=(q-shift-offset[:2])@np.linalg.inv(matrix[:2,:2]).T
        depth=uv@matrix[2,:2]+offset[2]
        maps.append(dict(uv=uv,depth=depth,matrix=matrix,offset=offset,side=side,face=0 if matrix[2,2]>0 else 1))
    return dict(row=row,rgba=rgba,q=q,target=target.ravel(),bright=bright.ravel(),body=bodyhit,maps=maps,shift=shift,shape=gx.shape)

def metrics(outline,rows,detail=False):
    stats=[]
    for c in rows:
        hits=[Polygon(outline[:,:2]).contains_points(m['uv']) for m in c['maps']]
        pred=c['body']|hits[0]|hits[1];miss=c['target']&~pred
        stats.append(dict(phase=c['row']['phase'],missing=int(miss.sum()),bright_missing=int((miss&c['bright']).sum()),extra=int((pred&~c['target']).sum()),source=int(c['target'].sum())))
    totals={k:sum(r[k] for r in stats) for k in ('missing','bright_missing','extra','source')}
    return (totals,stats) if detail else totals

def outline_from(x):
    result=WING.copy();result[1:6,:2]+=np.asarray(x).reshape(5,2);return result

def main():
    assert not OUT.exists();OUT.mkdir()
    assert sha(FIT)=='1e2fe4e6d01c746eaea3a57f393fb074f2a6bdb3f2a109b3b3a28ea5e877a7d6'
    fit=json.loads(FIT.read_text());rows=[prepare(r) for r in fit['rows']]
    before=metrics(WING,rows);assert before['missing']==344 and before['bright_missing']==70
    def loss(x):
        outline=outline_from(x)
        # Preserve clockwise convex wing fan and fixed roots; only five contour points move.
        edge=np.roll(outline[:,:2],-1,axis=0)-outline[:,:2]
        cross=edge[:,0]*np.roll(edge[:,1],-1)-edge[:,1]*np.roll(edge[:,0],-1)
        if np.any(cross<0):return 1e6
        m=metrics(outline,rows)
        return m['missing']+3*m['bright_missing']+m['extra']+12*np.sum(np.asarray(x)**2)
    result=differential_evolution(loss,[(-.65,.65)]*10,x0=np.zeros(10),seed=64107,popsize=5,maxiter=65,polish=False,workers=1,tol=.001)
    outline=outline_from(result.x);after,perphase=metrics(outline,rows,True)
    # A coordinate proposal cannot silently replace the original anatomy unless its weighted score improves.
    assert loss(result.x)<loss(np.zeros(10))
    np.savez(OUT/'proposal.npz',wing_outline=outline,parameters=np.array([r['parameters'] for r in fit['rows']]))
    contributions=[[] for _ in range(4)];ownership=[]
    for c,stat in zip(rows,perphase):
        candidates=[]
        for m in c['maps']:
            hit=Polygon(outline[:,:2]).contains_points(m['uv'])
            candidates.append(np.where(hit,m['depth'],-np.inf))
        # Camera-local +Z is the viewing side; body-overlap is conservatively unassigned.
        owner=np.argmax(candidates,axis=0);visible=np.maximum(*candidates)>-np.inf
        accepted=c['target']&visible&~c['body'];unassigned=c['target']&~accepted
        for i in np.flatnonzero(accepted):
            side=int(owner[i]);m=c['maps'][side];x,y=np.floor(c['q'][i]).astype(int)
            contributions[side*2+m['face']].append((m['uv'][i],c['rgba'][y,x,:3],abs(np.linalg.det(m['matrix'][:2,:2])),c['row']['phase'],[x,y]))
        ownership.append(dict(phase=stat['phase'],wing_pixels=int(accepted.sum()),body_or_unresolved_pixels=int(unassigned.sum()),missing_pixels=[c['q'][i].tolist() for i in np.flatnonzero(c['target']&~(visible|c['body']))]))
    atlas_report=[]
    for domain,samples in enumerate(contributions):
        w,h=64,96;sums=np.zeros((h,w,3));weights=np.zeros((h,w));counts=np.zeros((h,w),int)
        for uv,rgb,weight,phase,pixel in samples:
            x=int(np.floor(uv[0]/8*w));y=int(np.floor((uv[1]+7)/13*h))
            if 0<=x<w and 0<=y<h:
                sums[y,x]+=rgb*weight;weights[y,x]+=weight;counts[y,x]+=1
        known=weights>0;atlas=np.zeros((h,w,4),np.uint8);atlas[known,:3]=np.rint(sums[known]/weights[known,None]).astype(np.uint8);atlas[known,3]=255
        Image.fromarray(atlas).save(OUT/f'observed-wing-{domain}.png')
        atlas_report.append(dict(domain=domain,side=domain//2,face=domain%2,observations=len(samples),observed_texels=int(known.sum()),unknown_texels=int((~known).sum()),rule='Weighted repeated source observations only; holes transparent. Aggregate color is a proposal, not exact per-frame RGB. No synthesis, interpolation or opposite-face copying.'))
    for first in range(0,99,25):
        fig,axes=plt.subplots(5,5,figsize=(12,12),facecolor='#202020')
        for offset,ax in enumerate(axes.flat):
            phase=first+offset;ax.set_facecolor('#202020');ax.set_xticks([]);ax.set_yticks([])
            if phase>=99:ax.axis('off');continue
            c=rows[phase];h,w=c['rgba'].shape[:2];ax.imshow(c['rgba'],extent=[0,w,h,0],interpolation='nearest')
            for m,color in zip(c['maps'],('#ff66cc','#22ddff')):
                for shape,style,alpha in ((WING,':',.6),(outline,'-',1.)):
                    xy=shape[:,:2]@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift'];xy=np.vstack([xy,xy[0]])
                    ax.plot(xy[:,0],xy[:,1],style,color=color,lw=.8,alpha=alpha)
            ax.set_xlim(-3,w+3);ax.set_ylim(h+3,-3);s=perphase[phase]
            ax.set_title(f"{phase}: missing{s['missing']} bright{s['bright_missing']} extra{s['extra']}",fontsize=8,color='white')
        fig.tight_layout();fig.savefig(OUT/f'source-phases-{first:02}.png',dpi=100);plt.close(fig)
    report=dict(status='PRIVATE_FIXED_ANATOMY_PROPOSAL_REQUIRES_MODEL_REVIEW',source_fit=dict(path=str(FIT),sha256=sha(FIT)),recipe_sha256=sha(Path(__file__)),before=before,after=after,wing_outline=outline.tolist(),vertex_delta=(outline-WING).tolist(),motion_parameters_unchanged=True,source_clock=dict(phases=99,cycle_ticks=198,ticks_per_phase=2),source_hashes_checked=99,ownership=ownership,atlas_domains=atlas_report,source_rows=[r['source'] for r in fit['rows']],limits=['Single fixed anatomy for all99 phases; no per-frame resizing or source billboard.','Changes bounded to0.65units per coordinate with both wing roots fixed; hidden depth/head-tail remain inferred.','Old contact proof does not certify enlarged anatomy; all99 intervals need refreshed conservative receiver audit.','Observed atlas samples are sparse; their holes must not be reported as completed appearance.','This diagnostic is not a saved-model eight-view or motion review.','Per-phase alpha edge pixels can lie outside a center-ray outline; explicit residual coordinates retained.'])
    (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(before=before,after=after,atlas=atlas_report)),flush=True)
if __name__=='__main__':main()
