"""Regularize observed low-pass motion into a coherent cyclic cluster field."""
from pathlib import Path
import json,hashlib,sys
import numpy as np
from scipy.ndimage import gaussian_filter
from PIL import Image
from restart14_tree42_dense_motion import warp
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';DEST=BASE/('tree42-coherent-correspondence-v2'if '--fine'in sys.argv else 'tree42-coherent-correspondence-v1')
def main():
 DEST.mkdir(exist_ok=False);p=BASE/'tree42-smooth-correspondence-v1/flows.npz';packet=np.load(p);t=np.arange(8)/7;terminal=np.sum(packet['flow'][:8]*t[:,None,None,None],axis=0)/np.sum(t*t);sigma=1.5 if '--fine'in sys.argv else 2.5;terminal=gaussian_filter(terminal,(sigma,sigma,0));source=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1];x0,y0,w,h=map(int,packet['bbox']);features=[]
 for f in source['frames']:
  a=np.array(Image.open(f['path']).convert('RGBA'),dtype=float)/255;x,y,fw,fh=f['bbox'];c=np.zeros((h,w,4));c[y-y0:y-y0+fh,x-x0:x-x0+fw]=a;c[:,:,:3]*=c[:,:,3:];features.append(gaussian_filter(c,(2,2,0)))
 flow=np.stack([terminal*q for q in list(t)+list(t[6:0:-1])]);rows=[]
 for i,a in enumerate(flow):
  gy,gx=np.gradient(a,axis=(0,1));jac=(1+gx[:,:,0])*(1+gy[:,:,1])-gx[:,:,1]*gy[:,:,0];support=features[0][:,:,3]>.08;err=np.mean((features[0]-warp(features[i],a))**2,axis=2);zero=np.mean((features[0]-features[i])**2,axis=2);rows.append({'phase':i,'minimum_jacobian':float(jac.min()),'fold_pixels':int((jac<=0).sum()),'mean_fitted_feature_error':float(err[support].mean()),'mean_stationary_feature_error':float(zero[support].mean()),'maximum_step':float(np.linalg.norm(flow[(i+1)%14]-a,axis=2).max())});assert jac.min()>.1
 np.savez_compressed(DEST/'flows.npz',flow=flow.astype('float32'),bbox=packet['bbox']);(DEST/'report.json').write_text(json.dumps({'status':'COHERENT_FIT_REQUIRES_PHYSICAL_REVIEW','spatial_sigma_pixels':sigma,'parent_flow_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'rows':rows,'limits':['Linear forward/backward cluster displacement is fitted inference, not individually observed material-point correspondence.','Spatial regularization suppresses local folds caused by ambiguous screen stipple.','Depth/reverse displacement is unobserved; no texture modifications or added source support.']},indent=2)+'\n');print(rows)
if __name__=='__main__':main()
