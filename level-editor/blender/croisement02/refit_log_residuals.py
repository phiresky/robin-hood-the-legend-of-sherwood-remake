"""Bounded pixel-center projection fit of existing complete log bodies."""
import json,math
import numpy as np
from PIL import Image
from scipy.optimize import minimize
from scipy.spatial import ConvexHull
from catalog import OUT
from native_log_foreground_reference import sha
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def main():
    root=OUT/'state-target-evidence/log-trap';dest=root/'residual-fit-v1';dest.mkdir(exist_ok=False)
    prior=root/'applied-sloped-bank-hypothesis-v2.json';data=json.loads(prior.read_text());initial=np.array(data['survey']);rows=initial.copy()
    alpha=np.array(Image.open(root/'tick-089.png'))[:,:,3]>0;h,w=alpha.shape
    yy,xx=np.mgrid[:h*2,:w*2];points=np.column_stack(((xx.ravel()+.5)/2,(yy.ravel()+.5)/2));target=np.repeat(np.repeat(alpha,2,0),2,1).ravel()
    visible=np.array(Image.open(root/'native-order-reference/visible-log-phase0.png'))>0;visible=np.repeat(np.repeat(visible,2,0),2,1).ravel()
    angles=np.arange(16)*math.tau/16
    def raster(row):
        ax,ay,bx,by,r,za,zb=row;a=np.array([ax,-(ay+za*COS)/SIN,za]);b=np.array([bx,-(by+zb*COS)/SIN,zb]);axis=b-a;axis/=np.linalg.norm(axis);helper=[0,0,1]if abs(axis[2])<.9 else[1,0,0];u=np.cross(axis,helper);u/=np.linalg.norm(u);v=np.cross(axis,u);ring=r*(np.cos(angles)[:,None]*u+np.sin(angles)[:,None]*v);vertices=np.concatenate([a+ring,b+ring]);projected=np.column_stack((vertices[:,0],-vertices[:,1]*SIN-vertices[:,2]*COS));hull=ConvexHull(projected);low=projected.min(0);high=projected.max(0);indices=np.flatnonzero(np.all((points>=low)&(points<=high),axis=1));result=np.zeros(len(points),bool);result[indices]=np.all(points[indices]@hull.equations[:,:2].T+hull.equations[:,2]<=1e-8,axis=1);return result
    cache=[raster(row) for row in rows];before=np.logical_or.reduce(cache)
    # Keep complete bodies and constrain existing source axes; no temporal identity inferred.
    for iteration in range(2):
        for i in [7,9,8,5,3,6,0,1,2,4]:
            other=np.logical_or.reduce([p for j,p in enumerate(cache)if j!=i]);base=initial[i]
            def objective(values):
                row=base.copy();row[:5]=values;row[5:]+=values[4]-base[4];mask=other|raster(row)
                return (5*np.count_nonzero(target&~mask)+3*np.count_nonzero(visible&~mask)+.8*np.count_nonzero(mask&~target)+4*np.sum((values-base[:5])**2))
            bounds=[(v-2,v+2)for v in base[:4]]+[(base[4],base[4]+2)]
            fit=minimize(objective,rows[i,:5],method='Powell',bounds=bounds,options=dict(maxiter=4,maxfev=1000,xtol=.07,ftol=1e-5));rows[i,:5]=fit.x;rows[i,5:]=base[5:]+fit.x[4]-base[4];cache[i]=raster(rows[i])
    after=np.logical_or.reduce(cache)
    def metrics(mask):return dict(native_missing_samples=int((target&~mask).sum()),expected_visible_missing_samples=int((visible&~mask).sum()),inferred_continuation_samples=int((mask&~target).sum()))
    result=dict(status='bounded private fit; continuous contact and actual raster review required',prior_sha256=sha(prior),source_sha256=sha(root/'tick-089.png'),survey=rows.tolist(),before=metrics(before),after=metrics(after),bounds='Each endpoint coordinate +/-2 native pixels; radius +0..2; equal source-ray endpoint lift by radius increase. All ten closed bodies retained.',limitations=['Inferred continuation penalty is geometric only, not evidence of foreground ownership.','Analytical pixel-center projection must be compared against actual Blender mesh rendering.','No temporal identity or state approval.'])
    (dest/'survey.json').write_text(json.dumps(result,indent=2)+'\n')
    rgb=np.zeros((h*2,w*2,3),np.uint8);rgb.reshape(-1,3)[target&after]=(60,180,80);rgb.reshape(-1,3)[target&~after]=(255,20,230);rgb.reshape(-1,3)[after&~target]=(35,80,180);Image.fromarray(rgb).save(dest/'fit.png')
    print(json.dumps(result,indent=2))

if __name__=='__main__':main()
