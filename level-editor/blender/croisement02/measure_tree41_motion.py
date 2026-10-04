"""Measure source-plane motion from the tree's own native phases; no synthetic wind."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import uniform_filter,gaussian_filter
from catalog import OUT,tree_workspace

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 dest=OUT/'tree41-animation-proof';dest.mkdir(exist_ok=False)
 data=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'][3]
 packet=json.loads((OUT/'forest-v4-sources/tree-41/partition.json').read_text());x,y,w,h=packet['bbox'];margin=30;x-=margin;y-=margin;w+=margin*2;h+=margin*2
 def crop(frame):
  im=Image.new('RGBA',(w,h));im.alpha_composite(Image.open(frame['image']).convert('RGBA'),(frame['bbox'][0]-x,frame['bbox'][1]-y));return np.asarray(im,dtype=np.float32)/255
 arrays=[crop(f) for f in data['frames']];a=arrays[0];known=a[:,:,3]>.5;flows=[];reports=[]
 for i,b in enumerate(arrays):
  best=np.full((h,w),np.inf,np.float32);flow=np.zeros((h,w,2),np.float32)
  # Match RGB and physical alpha together in a 9px neighborhood. A missing target
  # is penalized, rather than matching every transparent black patch cheaply.
  for dy in range(-5,6):
   for dx in range(-5,6):
    shifted=np.zeros_like(b);ya,yb=max(0,-dy),min(h,h-dy);xa,xb=max(0,-dx),min(w,w-dx)
    shifted[ya:yb,xa:xb]=b[ya+dy:yb+dy,xa+dx:xb+dx]
    cost=uniform_filter(np.mean((a[:,:,:3]*a[:,:,3,None]-shifted[:,:,:3]*shifted[:,:,3,None])**2,axis=2)+.2*(a[:,:,3]-shifted[:,:,3])**2,size=9)
    replace=cost<best;best[replace]=cost[replace];flow[replace]=[dx,dy]
  support=uniform_filter(known.astype(np.float32),size=9)>.2
  confidence=support&(best<.04)
  if i==0:flow[:]=0;confidence=support
  weights=gaussian_filter(confidence.astype(np.float32),sigma=2)
  for axis in range(2):flow[:,:,axis]=gaussian_filter(flow[:,:,axis]*confidence,sigma=2)/np.maximum(weights,.0001)
  flow[weights<.1]=0
  flows.append(flow);reports.append(dict(phase=i,delay=data['frames'][i]['delay'],mean_error=float(best[support].mean()),confident_fraction=float(confidence[support].mean()),max_motion=float(np.linalg.norm(flow,axis=2).max()),source_frame_sha256=sha(Path(data['frames'][i]['image']))))
 np.savez_compressed(dest/'motion.npz',flow=np.stack(flows),bbox=np.array([x,y,w,h]),alpha=np.stack([b[:,:,3] for b in arrays]),rgb=np.stack([b[:,:,:3] for b in arrays]))
 Image.fromarray((arrays[0]*255).astype(np.uint8)).save(dest/'native-phase-0.png');Image.fromarray((arrays[7]*255).astype(np.uint8)).save(dest/'native-phase-7.png')
 result=dict(asset_id='croisement02-tree-41',native_animation=3,model=str(tree_workspace(41)/'model.blend'),model_sha256=sha(tree_workspace(41)/'model.blend'),source_bbox=[x,y,w,h],phases=reports,method='Local RGB+alpha minimum-error matching over +/-5 source pixels, 9px neighborhoods; confidence-weighted spatial smoothing. No invented sinusoidal wind.',limitations=['Source-plane correspondence is estimated, not uniquely recovered physics.','Native RGB and alpha changes are not fully explained by motion alone.','Hidden leaf motion inherits local observed material correspondence as explicit inference.'])
 (dest/'motion.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(reports))
if __name__=='__main__':main()
