"""Fit bounded fine leaf-patch correspondence with subpixel and fold guards."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image
from scipy.ndimage import uniform_filter,gaussian_filter,map_coordinates
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';DEST=BASE/'tree42-leaf-correspondence-v1';OFFSETS=[(x,y)for y in range(-3,4)for x in range(-3,4)]
def match(a,b):
 h,w=a.shape[:2];scores=[]
 for dx,dy in OFFSETS:
  shifted=np.zeros_like(b);x0=max(0,-dx);x1=min(w,w-dx);y0=max(0,-dy);y1=min(h,h-dy);shifted[y0:y1,x0:x1]=b[y0+dy:y1+dy,x0+dx:x1+dx];error=np.mean((a-shifted)**2,axis=2);scores.append(uniform_filter(error,size=5,mode='constant'))
 costs=np.stack(scores);best=costs.argmin(0);yy,xx=np.mgrid[:h,:w];center=costs[best,yy,xx];flow=np.array(OFFSETS,dtype=float)[best]
 for axis,stride in ((0,1),(1,7)):
  valid=np.abs(flow[:,:,axis])<3;minus=costs[np.clip(best-stride,0,48),yy,xx];plus=costs[np.clip(best+stride,0,48),yy,xx];den=2*(minus-2*center+plus);adjust=np.divide(minus-plus,den,out=np.zeros_like(center),where=valid&(den>1e-7));flow[:,:,axis]+=np.clip(adjust,-.45,.45)
 return flow,center,costs[24]
def main():
 DEST.mkdir(exist_ok=False);source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];x0,y0,w,h=616,688,342,288;frames=[]
 for f in source['frames'][:8]:
  im=np.array(Image.open(f['path']).convert('RGBA'),dtype=np.float32)/255;x,y,fw,fh=f['bbox'];a=np.zeros((h,w,4),np.float32);a[y-y0:y-y0+fh,x-x0:x-x0+fw]=im;a[:,:,:3]*=a[:,:,3:];a[:,:,3]*=1.5;frames.append(a)
 yy,xx=np.mgrid[:h,:w];flows=[np.zeros((h,w,2),np.float32)];rows=[]
 for phase in range(1,8):
  forward,error,zero=match(frames[0],frames[phase]);backward,_,_=match(frames[phase],frames[0]);returned=np.stack([map_coordinates(backward[:,:,c],[yy+forward[:,:,1],xx+forward[:,:,0]],order=1,mode='nearest')for c in range(2)],axis=2);valid=(np.linalg.norm(forward+returned,axis=2)<1)&(uniform_filter(frames[0][:,:,3],size=5)>.08)&(error<=zero+1e-5);field=gaussian_filter(forward*valid[:,:,None],(.85,.85,0))
  # Dampen locally folded mappings, without inventing uncovered texture pixels.
  for _ in range(10):
   gy,gx=np.gradient(field,axis=(0,1));jac=(1+gx[:,:,0])*(1+gy[:,:,1])-gx[:,:,1]*gy[:,:,0];bad=gaussian_filter((jac<.3).astype(float),.7);field*=1-.45*bad[:,:,None]
  inverse=-field.copy()
  for _ in range(16):inverse=-np.stack([map_coordinates(field[:,:,c],[yy+inverse[:,:,1],xx+inverse[:,:,0]],order=1,mode='nearest')for c in range(2)],axis=2)
  alpha=map_coordinates(frames[0][:,:,3]/1.5,[yy+inverse[:,:,1],xx+inverse[:,:,0]],order=1,mode='constant')>=.5;expected=frames[phase][:,:,3]>.5;gy,gx=np.gradient(field,axis=(0,1));jac=(1+gx[:,:,0])*(1+gy[:,:,1])-gx[:,:,1]*gy[:,:,0];row={'phase':phase,'accepted_centers':int(valid.sum()),'proxy_missing':int((expected&~alpha).sum()),'proxy_extra':int((~expected&alpha).sum()),'max_motion':float(np.linalg.norm(field,axis=2).max()),'minimum_jacobian':float(jac.min()),'fold_pixels':int((jac<=0).sum())};rows.append(row);flows.append(field.astype('float32'));print(row,flush=True)
 flows+=list(reversed(flows[1:7]));np.savez_compressed(DEST/'flows.npz',flow=np.stack(flows),bbox=np.array([x0,y0,w,h]));(DEST/'report.json').write_text(json.dumps({'status':'LEAF_PATCH_FIT_REQUIRES_PHYSICAL_VALIDATION','source_sha256':hashlib.sha256((BASE/'source-reconciliation-v1/report.json').read_bytes()).hexdigest(),'rows':rows,'limits':['Image-warp proxy is not rendered physical coverage.','Five-pixel patch support; subpixel interpolation and hidden/rear motion remain inferred.','No texture, source image or wood modifications.']},indent=2)+'\n')
if __name__=='__main__':main()
