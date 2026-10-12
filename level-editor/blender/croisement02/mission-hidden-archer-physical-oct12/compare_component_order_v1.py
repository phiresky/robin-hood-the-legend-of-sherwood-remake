"""Compare a declared static-wood-before-patch source role using untrimmed samples."""
import sys,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import classify_actual_canopy05_v1 as c
from operational_records import read_record
q=c.q;dest=q.OUT/'component-source-order-candidate-v1';assert not dest.exists();dest.mkdir()
_,full=c.load(c.FULL);_,static=c.load(c.STATIC);_,moving=c.load(c.MOVING)
qualification=read_record(q.OUT/'qualification.json');native=read_record(q.OUT/'profile02-native-loop-composition-v1/report.json')['loop_frames'];bg=Image.open(q.ROOT/qualification['background']['path']).convert('RGBA');profiles=[]
for n,crop in [(1,[79,83,240,206]),(2,[411,0,641,136])]:
 x,y,x1,y1=crop;w,h=x1-x,y1-y;sheet=Image.new('RGB',(w*6,(h+32)*4),(20,20,20));draw=ImageDraw.Draw(sheet);row=qualification['rows'][n-1];states=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames'];records=[]
 def layer(array):
  out=Image.new('RGBA',(w,h));out.alpha_composite(Image.fromarray(array),(104-x,-y));return out
 for j,(phase,f) in enumerate(zip(['initial','transition-0','transition-1','applied'],states)):
  patch=Image.new('RGBA',(w,h));patch.alpha_composite(q.rgba(f),(int(f['top_left'][0])-x,int(f['top_left'][1])-y));pm=np.array(patch)[:,:,3]>0;frames=[]
  for i in range(14):
   nl=Image.new('RGBA',(w,h));nf=native[i];nl.alpha_composite(q.rgba(nf),(nf['top_left'][0]-x,nf['top_left'][1]-y));nm=np.array(nl)[:,:,3]>0;sl=layer(static[i][1]);ml=layer(moving[i][1]);fl=layer(full[i][1]);ma=np.array(ml)[:,:,3]>0;fa=np.array(fl)[:,:,3]>0
   frames.append(dict(frame=i,native_visible_patch=int((pm&~nm).sum()),whole_owner_extra=int((pm&~nm&fa).sum()),candidate_moving_only_extra=int((pm&~nm&ma).sum())))
   if i in [0,7]:
    back=bg.crop(crop);orig=Image.alpha_composite(Image.alpha_composite(back,patch),nl);old=Image.alpha_composite(Image.alpha_composite(back,patch),fl);candidate=Image.alpha_composite(Image.alpha_composite(Image.alpha_composite(back,sl),patch),ml)
    col=0 if i==0 else 3
    for k,(im,title) in enumerate([(orig,'Original source order'),(old,'Whole owner (incorrect wood)'),(candidate,'Static wood / patch / crown')]):
     sheet.paste(im.convert('RGB'),((col+k)*w,j*(h+32)+32));draw.text(((col+k)*w+2,j*(h+32)+2),f'{phase} frame{i}',fill='white');draw.text(((col+k)*w+2,j*(h+32)+16),title,fill='white')
  records.append(dict(phase=phase,frames=frames))
 sheet.save(dest/f'profile{n:02}-source-order.png');profiles.append(dict(profile=n,crop=crop,states=records))
report=dict(status='FOCUSED_SOURCE_ORDER_CANDIDATE_NO_GEOMETRY_OR_RUNTIME_EDITS',profiles=profiles,manifests={str(p/'manifest.json'):c.sha(p/'manifest.json') for p in [c.FULL,c.STATIC,c.MOVING]},candidate='Same physical samples, full staticwood role before zero-elevation patch, moving crown role after patch. No pixel-alpha crop or geometry deletion.',limits=['Moving crown contains mixed native/static foliage, so this does not qualify crown ownership.','Background still uses original artwork for source-context diagnosis; final physical receiver integration remains pending.','Initial and terminal source order verified from behavior; no globalwholecrown role transfer authorized.'])
(dest/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([dict(profile=r['profile'],ranges=[dict(phase=s['phase'],moving_extra=[min(f['candidate_moving_only_extra'] for f in s['frames']),max(f['candidate_moving_only_extra'] for f in s['frames'])]) for s in r['states']]) for r in profiles]))
