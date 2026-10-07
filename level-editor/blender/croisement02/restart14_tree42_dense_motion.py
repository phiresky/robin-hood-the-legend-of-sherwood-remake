"""Fit bounded subpixel canopy motion with bidirectional photometric checks."""
from pathlib import Path
import json,hashlib,sys
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import gaussian_filter,map_coordinates,zoom
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';DEST=BASE/('tree42-smooth-correspondence-v1'if '--smooth'in sys.argv else 'tree42-dense-correspondence-v1')
def warp(a,flow):
 yy,xx=np.mgrid[:a.shape[0],:a.shape[1]];coords=[yy+flow[:,:,1],xx+flow[:,:,0]];return np.stack([map_coordinates(a[:,:,c],coords,order=1,mode='constant',cval=0)for c in range(a.shape[2])],axis=2)
def solve(a,b):
 flow=None
 for scale in (4,2,1):
  aa=zoom(a,(1/scale,1/scale,1),order=1);bb=zoom(b,(1/scale,1/scale,1),order=1)
  if flow is None:flow=np.zeros((*aa.shape[:2],2),np.float32)
  else:flow=zoom(flow,(aa.shape[0]/flow.shape[0],aa.shape[1]/flow.shape[1],1),order=1)*2
  gy,gx=np.gradient(bb,axis=(0,1))
  for _ in range(12):
   shifted=warp(bb,flow);ix=warp(gx,flow);iy=warp(gy,flow);error=aa-shifted
   smooth=lambda x:gaussian_filter(x,1.8)
   xx=smooth(np.sum(ix*ix,axis=2))+.0002;yy=smooth(np.sum(iy*iy,axis=2))+.0002;xy=smooth(np.sum(ix*iy,axis=2));ex=smooth(np.sum(ix*error,axis=2));ey=smooth(np.sum(iy*error,axis=2));det=xx*yy-xy*xy
   step=np.stack([(yy*ex-xy*ey)/det,(xx*ey-xy*ex)/det],axis=2);step=np.clip(step,-.45,.45);step=gaussian_filter(step,(.65,.65,0));flow=np.clip(flow+step,-4/scale,4/scale)
 return flow

def main():
 DEST.mkdir(exist_ok=False);report=json.loads((BASE/'source-reconciliation-v1/report.json').read_text());group=report['groups'][1];x0,y0,w,h=616,688,342,288;frames=[]
 for f in group['frames']:
  a=np.array(Image.open(f['path']).convert('RGBA'));x,y,fw,fh=f['bbox'];canvas=np.zeros((h,w,4),np.float32);canvas[y-y0:y-y0+fh,x-x0:x-x0+fw]=a/255;frames.append(canvas)
 features=[gaussian_filter(np.concatenate([a[:,:,:3]*a[:,:,3:],a[:,:,3:]],axis=2),((2.,2.,0)if '--smooth'in sys.argv else(.65,.65,0)))for a in frames];flows=[];rows=[]
 for phase in range(8):
  forward=solve(features[0],features[phase])if phase else np.zeros((h,w,2),np.float32);backward=solve(features[phase],features[0])if phase else forward.copy();returned=warp(backward,forward);roundtrip=np.linalg.norm(forward+returned,axis=2);error=np.mean((features[0]-warp(features[phase],forward))**2,axis=2);zero=np.mean((features[0]-features[phase])**2,axis=2);support=gaussian_filter(frames[0][:,:,3],2)>0.08;confidence=support&(roundtrip<.8)&(error<=zero+0.0004)
  # Reject ambiguous motion; smoothly attenuate its boundary instead of a hard tear.
  weight=gaussian_filter(confidence.astype('float32'),1);field=forward*weight[:,:,None];flows.append(field.astype('float32'));rows.append({'phase':phase,'supported_pixels':int(support.sum()),'consistent_pixels':int(confidence.sum()),'mean_motion':float(np.linalg.norm(field[support],axis=1).mean()),'max_motion':float(np.linalg.norm(field,axis=2).max()),'mean_roundtrip_supported':float(roundtrip[support].mean()),'fitted_error_supported':float(error[support].mean()),'stationary_error_supported':float(zero[support].mean())});print(rows[-1],flush=True)
 flows+=list(reversed(flows[1:7]));rows+=[dict(rows[14-i],phase=i)for i in range(8,14)];np.savez_compressed(DEST/'flows.npz',flow=np.stack(flows),bbox=np.array([x0,y0,w,h]));(DEST/'report.json').write_text(json.dumps({'status':'SUBPIXEL_CORRESPONDENCE_CANDIDATE','source_report_sha256':hashlib.sha256((BASE/'source-reconciliation-v1/report.json').read_bytes()).hexdigest(),'rows':rows,'bbox':[x0,y0,w,h],'constraints':['Frozen phase0 zero motion; native palindrome matched by identical source frames.','Bidirectional0.8pixel consistency and photometric non-regression gate; smooth inferred interpolation between image constraints.','All motion lies in observed image plane; depth and rear motion remain unobserved.']},indent=2)+'\n')
if __name__=='__main__':main()
