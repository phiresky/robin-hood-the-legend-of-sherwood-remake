"""Measure conservative image-plane patch correspondence without changing artwork."""
from pathlib import Path
import json,hashlib
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';DEST=OUT/'tree42-correspondence-v1'
def main():
 DEST.mkdir(exist_ok=False);report=json.loads((OUT/'source-reconciliation-v1/report.json').read_text());g=report['groups'][1];box=[616,688,342,288];x0,y0,w,h=box;frames=[]
 for f in g['frames']:
  a=np.array(Image.open(f['path']).convert('RGBA'));x,y,fw,fh=f['bbox'];canvas=np.zeros((h,w,4),np.uint8);canvas[y-y0:y-y0+fh,x-x0:x-x0+fw]=a;frames.append(canvas)
 features=[np.concatenate([a[:,:,:3]/255*(a[:,:,3:]/255),a[:,:,3:]/255],axis=2).astype('float32')for a in frames]
 offsets=[(dx,dy)for dy in range(-6,7)for dx in range(-6,7)];radius=10
 def match(a,b,x,y):
  patch=a[y-radius:y+radius+1,x-radius:x+radius+1];scores=[]
  for dx,dy in offsets:
   q=b[y+dy-radius:y+dy+radius+1,x+dx-radius:x+dx+radius+1];scores.append(float(np.mean((patch-q)**2)))
  k=int(np.argmin(scores));dx,dy=offsets[k];second=min(v for o,v in zip(offsets,scores)if abs(o[0]-dx)>1 or abs(o[1]-dy)>1);return dx,dy,scores[k],scores[offsets.index((0,0))],second
 rows=[];sheet=Image.new('RGB',(w*4,(h+25)*4),'#eee');draw=ImageDraw.Draw(sheet)
 for phase,a in enumerate(frames):
  samples=[]
  for y in range(20,h-20,14):
   for x in range(20,w-20,14):
    if np.count_nonzero(frames[0][y-radius:y+radius+1,x-radius:x+radius+1,3])<45:continue
    dx,dy,cost,zero,second=match(features[0],features[phase],x,y)
    bx,by,_,_,_=match(features[phase],features[0],x+dx,y+dy)
    accepted=abs(dx+bx)<=1 and abs(dy+by)<=1 and cost<=.12 and (cost==0 or second>=cost*1.08)
    samples.append({'pixel':[x+x0,y+y0],'delta':[dx,dy],'cost':cost,'zero_cost':zero,'second_cost':second,'roundtrip':[dx+bx,dy+by],'accepted':bool(accepted)})
  accepted=[s for s in samples if s['accepted']];rows.append({'phase':phase,'samples':samples,'accepted':len(accepted),'tested':len(samples),'mean_delta':np.mean([s['delta']for s in accepted],axis=0).tolist()if accepted else None})
  rgb=Image.new('RGB',(w,h),'#303030');rgb.paste(Image.fromarray(a),mask=Image.fromarray(a[:,:,3]));ox=(phase%4)*w;oy=(phase//4)*(h+25);sheet.paste(rgb,(ox,oy+25));draw.text((ox+5,oy+5),f'Phase {phase}: {len(accepted)}/{len(samples)} consistent',fill='black')
  for s in accepted:
   x,y=s['pixel'];dx,dy=s['delta'];px=ox+x-x0;py=oy+25+y-y0;draw.line((px,py,px+dx*3,py+dy*3),fill='#00ffff',width=1)
 sheet.save(DEST/'correspondence-sheet.png');result={'status':'DIAGNOSTIC_ONLY','source_report_sha256':hashlib.sha256((OUT/'source-reconciliation-v1/report.json').read_bytes()).hexdigest(),'bbox':box,'patch_radius':radius,'search_radius':6,'rows':rows,'limitations':['Image-plane correspondence, not 3D depth recovery.','Rejected or unobserved surface motion is not measured.','Cyan arrows are magnified3x; source images unchanged.']};(DEST/'report.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['phase'],r['accepted'],r['tested'],r['mean_delta'])for r in rows])
if __name__=='__main__':main()
