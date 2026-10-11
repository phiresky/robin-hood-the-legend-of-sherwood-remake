"""Select one best-facing source observation per wing texel without RGB averaging."""
import json
from pathlib import Path
import numpy as np
from PIL import Image
from matplotlib.path import Path as Polygon
from scipy.spatial import ConvexHull
from restart64_butterfly07_source_ownership import prepare,sha,ROOT,WING,geometry
FIT=ROOT/'level-editor/work/croisement02-refinement/restart64-butterfly07-pose-refinement-v1/fit.json'
OUT=ROOT/'level-editor/work/croisement02-refinement/restart65-butterfly07-exact-observation-atlas-v1'

def main():
    assert not OUT.exists();OUT.mkdir();fit=json.loads(FIT.read_text());w,h=64,96
    yy,xx=np.mgrid[:h,:w];uv=np.c_[(xx.ravel()+.5)/w*8,(yy.ravel()+.5)/h*13-7];domain=Polygon(WING[:,:2]).contains_points(uv)
    sums=np.zeros((4,h*w,3));squares=sums.copy();weights=np.zeros((4,h*w));counts=np.zeros((4,h*w),int);records=[];best_weight=np.zeros((4,h*w));best_rgb=np.zeros((4,h*w,3),np.uint8);best_phase=np.full((4,h*w),-1,int)
    for row in fit['rows']:
        c=prepare(row);a=c['rgba'];height,width=a.shape[:2];body,_=geometry(row['parameters']);b=body[:,:2]+c['shift'];bodypolygon=Polygon(b[ConvexHull(b).vertices])
        for m in c['maps']:
            xy=uv@m['matrix'][:2,:2].T+m['offset'][:2]+c['shift'];ix=np.floor(xy[:,0]).astype(int);iy=np.floor(xy[:,1]).astype(int)
            valid=domain&(ix>=0)&(ix<width)&(iy>=0)&(iy<height)&~bodypolygon.contains_points(xy)
            other=c['maps'][1-m['side']];otheruv=(xy-c['shift']-other['offset'][:2])@np.linalg.inv(other['matrix'][:2,:2]).T
            ownz=uv@m['matrix'][2,:2]+m['offset'][2];otherz=otheruv@other['matrix'][2,:2]+other['offset'][2]
            valid&=~(Polygon(WING[:,:2]).contains_points(otheruv)&(otherz>ownz))
            ids=np.flatnonzero(valid);ids=ids[a[iy[ids],ix[ids],3]>0]
            # Footprint coverage is observed; unknown texels stay gray, never smeared or mirrored.
            rgb=a[iy[ids],ix[ids],:3].astype(float);weight=abs(np.linalg.det(m['matrix'][:2,:2]))**2;d=m['side']*2+m['face']
            better=ids[weight>best_weight[d,ids]];best_weight[d,better]=weight;best_rgb[d,better]=a[iy[better],ix[better],:3];best_phase[d,better]=row['phase']
            sums[d,ids]+=rgb*weight;squares[d,ids]+=rgb**2*weight;weights[d,ids]+=weight;counts[d,ids]+=1
            records.append(dict(phase=row['phase'],side=m['side'],face=m['face'],accepted_texels=len(ids),projected_area_weight=weight))
    result=[]
    for d in range(4):
        known=weights[d]>0;atlas=np.full((h*w,4),255,np.uint8);atlas[:,:3]=112;atlas[known,:3]=best_rgb[d,known]
        mask=np.zeros(h*w,np.uint8);mask[known]=255;mask[~domain]=0
        np.save(OUT/f'wing-{d}-source-phase.npy',best_phase[d].reshape(h,w))
        Image.fromarray(atlas.reshape(h,w,4)).save(OUT/f'wing-{d}.png');Image.fromarray(mask.reshape(h,w)).save(OUT/f'wing-{d}-known.png')
        variance=squares[d,known]/weights[d,known,None]-(sums[d,known]/weights[d,known,None])**2
        result.append(dict(domain=d,wing=d//2,face=d%2,known_texels=int((known&domain).sum()),wing_texels=int(domain.sum()),unknown_wing_texels=int((~known&domain).sum()),mean_observation_channel_std=float(np.sqrt(np.maximum(variance,0)).mean()),image_sha256=sha(OUT/f'wing-{d}.png')))
    report=dict(status='OBSERVED_ONLY_TEXTURE_PROPOSAL_NOT_APPROVED',fit_sha256=sha(FIT),recipe_sha256=sha(Path(__file__)),resolution=[w,h],uv_domain=[0,8,-7,6],domains=result,observations=records,limits=['Only source07 used. Front and rear domains are never copied between wings or faces.','Every known texel copies one exact source RGB from its most front-facing unoccluded observation. Per-texel source phase retained. This source mosaic still cannot reproduce all differently shaded frames.','Unknown wing texels and body remain neutral gray; no synthesized texture.','Body-overlap pixels conservatively excluded from wing ownership.','Greatest projected-area observation selected; first source phase wins exact ties. No averaging, interpolation or synthesis.'])
    (OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
