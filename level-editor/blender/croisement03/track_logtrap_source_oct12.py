"""Produce inspectable local artwork tracks, without claiming physical identity."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_erosion
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/active/croisement03/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=WORK/'motion-tracks-v1';out.mkdir(exist_ok=False);manifest=WORK/'motion-source-v1/manifest.json';m=json.loads(manifest.read_text());frames=sorted((f for f in m['frames'] if f['opaque_wood_pixels']),key=lambda f:min(f['updates']));initial=WORK/'initial-aligned.png';previous=np.array(Image.open(initial).convert('RGB'),dtype=float).mean(axis=2);anchors=[]
 for log in [9,7,4,0]:
  mask=np.array(Image.open(WORK/f'owned-candidate-v2/initial/owner-{log:02}.png'))[:,:,3]>=128;eroded=binary_erosion(mask,iterations=3);yy,xx=np.where(eroded);assert len(xx)>20
  for quantile in [.25,.5,.75]:
   x=np.quantile(xx,quantile);eligible=np.where(abs(xx-x)<2)[0];y=np.median(yy[eligible]);k=eligible[np.argmin((xx[eligible]-x)**2+(yy[eligible]-y)**2)];anchors.append({'initial_log':log,'x':int(xx[k]),'y':int(yy[k]),'initial':[int(xx[k]),int(yy[k])]})
 rows=[];colors={9:'#ffcf40',7:'#30e0ff',4:'#ff60aa',0:'#70ff70'}
 for index,f in enumerate(frames):
  p=Path(f['path']);assert sha(p)==f['sha256'];rgba=np.array(Image.open(p).convert('RGBA'));current=rgba[:,:,:3].mean(axis=2);hits=[]
  for a in anchors:
   x,y=a['x'],a['y'];patch=previous[y-3:y+4,x-3:x+4];v=patch-patch.mean();vn=np.linalg.norm(v);options=[]
   for dy in range(-7,8):
    for dx in range(-7,8):
     nx,ny=x+dx,y+dy
     if nx<3 or ny<3 or nx>=297 or ny>=237 or rgba[ny,nx,3]<128:continue
     q=current[ny-3:ny+4,nx-3:nx+4];w=q-q.mean();den=vn*np.linalg.norm(w);ncc=float((v*w).sum()/den) if den>1e-8 else -1
     options.append((ncc-.002*(dx*dx+dy*dy),ncc,nx,ny))
   options.sort(reverse=True);best=options[0];a['x'],a['y']=best[2:];hits.append({'initial_log':a['initial_log'],'initial_pixel':a['initial'],'pixel':[a['x'],a['y']],'native_pixel':[a['x']+420,a['y']+380],'ncc':best[1],'local_margin':best[0]-options[1][0]})
  im=Image.fromarray(rgba).convert('RGB').resize((900,720),Image.Resampling.NEAREST);d=ImageDraw.Draw(im)
  for h in hits:
   x,y=h['pixel'];c=colors[h['initial_log']];d.ellipse((x*3-4,y*3-4,x*3+4,y*3+4),outline=c,width=2)
  image=out/f'{index:02}-{p.stem}.png';im.save(image);rows.append({'profile':f['profile'],'frame':f['frame'],'updates':f['updates'],'anchors':hits,'image':str(image),'sha256':sha(image)});previous=current
 report={'status':'UNAPPROVED_LOCAL_FEATURE_TRACKS_NOT_PHYSICAL_IDENTITIES','source_manifest_sha256':sha(manifest),'initial_sha256':sha(initial),'rows':rows,'limitations':['Local7x7grayscale correlation can switch between repeated bark features.','NCC and margins are diagnostics; all proposed trajectories require visual inspection.','Only four initially exposed bodies are sampled; hidden six logs and chips remain untracked.','No rig or endpoint correspondence is asserted.']};(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('frames',len(rows),'minimum NCC',min(h['ncc'] for r in rows for h in r['anchors']))
if __name__=='__main__':main()
