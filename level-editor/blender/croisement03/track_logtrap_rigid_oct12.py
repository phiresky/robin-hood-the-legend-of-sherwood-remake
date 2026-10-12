"""Produce inspectable local artwork tracks, without claiming physical identity."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_erosion
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=WORK/'motion-rigid-tracks-v2';out.mkdir(exist_ok=False);manifest=WORK/'motion-source-v1/manifest.json';m=json.loads(manifest.read_text());frames=sorted((f for f in m['frames'] if f['opaque_wood_pixels']),key=lambda f:min(f['updates']));initial=WORK/'initial-aligned.png';previous=np.array(Image.open(initial).convert('RGB'),dtype=float).mean(axis=2);anchors=[]
 for log in [9,7,4,0]:
  mask=np.array(Image.open(WORK/f'owned-candidate-v2/initial/owner-{log:02}.png'))[:,:,3]>=128;eroded=binary_erosion(mask,iterations=3);yy,xx=np.where(eroded);assert len(xx)>20
  for quantile in [.25,.5,.75]:
   x=np.quantile(xx,quantile);eligible=np.where(abs(xx-x)<2)[0];y=np.median(yy[eligible]);k=eligible[np.argmin((xx[eligible]-x)**2+(yy[eligible]-y)**2)];anchors.append({'initial_log':log,'x':int(xx[k]),'y':int(yy[k]),'initial':[int(xx[k]),int(yy[k])]})
 rows=[];colors={9:'#ffcf40',7:'#30e0ff',4:'#ff60aa',0:'#70ff70'}
 for index,f in enumerate(frames):
  p=Path(f['path']);assert sha(p)==f['sha256'];rgba=np.array(Image.open(p).convert('RGBA'));current=rgba[:,:,:3].mean(axis=2);hits=[]
  for log in [9,7,4,0]:
   group=[a for a in anchors if a['initial_log']==log];points=np.array([[a['x'],a['y']] for a in group],float);center=points.mean(axis=0);patches=[]
   for x,y in points.astype(int):
    v=previous[y-3:y+4,x-3:x+4];v=v-v.mean();patches.append(v/max(1e-9,np.linalg.norm(v)))
   options=[]
   for angle in [-.06,0,.06]:
    rot=np.array([[np.cos(angle),-np.sin(angle)],[np.sin(angle),np.cos(angle)]])
    pp=(points-center)@rot.T+center
    for dy in range(-6,7):
     for dx in range(-6,7):
      moved=np.rint(pp+[dx,dy]).astype(int);scores=[]
      for template,(nx,ny) in zip(patches,moved):
       if nx<3 or ny<3 or nx>=297 or ny>=237 or rgba[ny,nx,3]<128:scores.append(-1);continue
       q=current[ny-3:ny+4,nx-3:nx+4];w=q-q.mean();scores.append(float((template*w).sum()/max(1e-9,np.linalg.norm(w))))
      score=float(np.mean(scores))-.002*(dx*dx+dy*dy)-.03*abs(angle);options.append((score,scores,moved.tolist(),angle))
   options.sort(key=lambda x:x[0],reverse=True);best=options[0]
   for a,xy,ncc in zip(group,best[2],best[1]):
    a['x'],a['y']=xy;hits.append({'initial_log':log,'initial_pixel':a['initial'],'pixel':xy,'native_pixel':[xy[0]+420,xy[1]+380],'ncc':ncc,'group_angle_step':best[3],'local_margin':best[0]-options[1][0]})
  im=Image.fromarray(rgba).convert('RGB').resize((900,720),Image.Resampling.NEAREST);d=ImageDraw.Draw(im)
  for h in hits:
   x,y=h['pixel'];c=colors[h['initial_log']];d.ellipse((x*3-4,y*3-4,x*3+4,y*3+4),outline=c,width=2)
  image=out/f'{index:02}-{p.stem}.png';im.save(image);rows.append({'profile':f['profile'],'frame':f['frame'],'updates':f['updates'],'anchors':hits,'image':str(image),'sha256':sha(image)});previous=current
 report={'status':'UNAPPROVED_LOCAL_FEATURE_TRACKS_NOT_PHYSICAL_IDENTITIES','source_manifest_sha256':sha(manifest),'initial_sha256':sha(initial),'rows':rows,'limitations':['Three correlated anchors per body share one local rigid translation/rotation; repeated textures can still produce incorrect body tracking.','NCC and margins are diagnostics; all proposed trajectories require visual inspection.','Only four initially exposed bodies are sampled; hidden six logs and chips remain untracked.','No rig or endpoint correspondence is asserted.']};(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('frames',len(rows),'minimum NCC',min(h['ncc'] for r in rows for h in r['anchors']))
if __name__=='__main__':main()
