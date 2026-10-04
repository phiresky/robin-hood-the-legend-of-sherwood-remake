"""Fit bounded coherent axes using their actual sloped cylinder projection."""
import hashlib,json,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import minimize
from scipy.spatial import ConvexHull
from catalog import OUT
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def main():
    root=OUT/'state-target-evidence/log-trap';source=root/'tick-089.png';alpha=np.array(Image.open(source))[:,:,3]>127;h,w=alpha.shape
    support_path=root/'applied-coherent-bank-slopes-v4.json';support=json.loads(support_path.read_text());sloped=np.array(support['survey']);initial=sloped[:,:5].copy()
    masks=json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks'];potential=np.zeros_like(alpha);bindings=[]
    # Measured mask overlaps do not establish front-to-back ownership.
    for m in masks:
        if m['index'] not in (5,6,29,30,62,63,130,133):continue
        x,y=m['box_top_left'];mw,mh=m['box_size'];l=max(x,390);t=max(y,441);r=min(x+mw,390+w);b=min(y+mh,441+h)
        if r<=l or b<=t:continue
        path=OUT/'baseline/masks'/m['png'];a=np.array(Image.open(path).convert('L'))[t-y:b-y,l-x:r-x]>0;potential[t-441:b-441,l-390:r-390]|=a;bindings.append(dict(global_mask=m['index'],layer_local=m['layer_index'],sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    angles=np.arange(16)*math.tau/16
    def raster(values):
        image=Image.new('L',(w,h));draw=ImageDraw.Draw(image)
        for index,(ax,ay,bx,by,r) in enumerate(values.reshape(-1,5)):
            za,zb=sloped[index,-2:];a=np.array([ax,-(ay+za*COS)/SIN,za]);b=np.array([bx,-(by+zb*COS)/SIN,zb]);axis=b-a;axis/=np.linalg.norm(axis);u=np.cross(axis,[0,0,1]);u/=np.linalg.norm(u);v=np.cross(axis,u);ring=r*(np.cos(angles)[:,None]*u+np.sin(angles)[:,None]*v);world=np.concatenate([a+ring,b+ring]);points=np.column_stack([world[:,0],-world[:,1]*SIN-world[:,2]*COS]);hull=ConvexHull(points);draw.polygon([tuple(v)for v in points[hull.vertices]],fill=255)
        return np.array(image)>0
    def objective(values):
        actual=raster(values);miss=(alpha&~actual).sum();unsupported=(actual&~alpha&~potential).sum();inferred=(actual&~alpha&potential).sum();regularizer=np.mean(((values.reshape(-1,5)-initial)/[6,6,6,6,2])**2)*.003
        return (8*miss+2*unsupported+.02*inferred)/alpha.sum()+regularizer
    bounds=[]
    for row in initial:bounds.extend([(v-3,v+3)for v in row[:4]]+[(max(2,row[4]-1),row[4]+1.5)])
    result=minimize(objective,initial.ravel(),method='Powell',bounds=bounds,options=dict(maxiter=8,xtol=.12,ftol=1e-5,maxfev=18000));actual=raster(result.x)
    survey=[[*map(float,row),*map(float,sloped[i,-2:])]for i,row in enumerate(result.x.reshape(-1,5))]
    report=dict(status='unapproved bounded sloped silhouette fit; support must be recomputed',support_manifest_sha256=hashlib.sha256(support_path.read_bytes()).hexdigest(),source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),survey=survey,mask_bindings=bindings,metrics=dict(native_coverage=float((alpha&actual).sum()/alpha.sum()),inferred_under_potential_foreground=int((actual&~alpha&potential).sum()),unsupported_projection=int((actual&~alpha&~potential).sum())),limitations=['Mask overlap is potential ownership only; actual geometry and native depth sorting must establish foreground occlusion.','Native transparent gaps remain evidence, not automatically wood fractures or approved inferred geometry.','No per-log temporal identity claim; independent final endpoint survey only.'])
    (root/'applied-sloped-log-fit-v1.json').write_text(json.dumps(report,indent=2)+'\n');rgb=np.zeros((h,w,3),dtype=np.uint8);rgb[alpha&actual]=(70,200,90);rgb[alpha&~actual]=(240,40,180);rgb[actual&~alpha&potential]=(40,100,230);rgb[actual&~alpha&~potential]=(240,160,30);Image.fromarray(rgb).resize((w*3,h*3),Image.Resampling.NEAREST).save(root/'applied-sloped-log-fit-v1.png');print(report['metrics'])
if __name__=='__main__':main()
