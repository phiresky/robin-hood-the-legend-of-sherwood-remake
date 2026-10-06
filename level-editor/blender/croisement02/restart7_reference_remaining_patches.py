from pathlib import Path
import json,hashlib,math
import numpy as np
from PIL import Image
b=Path('level-editor/work/croisement02-refinement/restart7-source-patch-delivery/contracts-v1');lib=Path('level-editor/library');manifest=json.loads((b/'manifest.json').read_text());resources={r['path']:Path(r['source']) for r in manifest['resources']};images={}
def image(f):
 p=f['path']
 if p not in images:
  q=resources.get(p,lib/p);data=q.read_bytes();assert hashlib.sha256(data).hexdigest()==f['sha256'];images[p]=np.array(Image.open(q).convert('RGBA'))
 return images[p]
def frame(frames,tick,loop=True):
 if not frames:return None
 duration=sum(f['delay']+1 for f in frames);tick=tick%duration if loop else min(tick,duration-1)
 for f in frames:
  if tick<f['delay']+1:return f
  tick-=f['delay']+1
 raise AssertionError()
def phase_frame(p,tick):
 if tick<0:return frame(p['initial'],0,p['initial_loop'])
 # Counter simulation deliberately independent of the preview's duration arithmetic.
 row=p['transition'];index=0;counter=0;applied=False
 for _ in range(tick):
  counter+=1
  if counter>row[index]['delay']:counter=0;index+=1
  if index>=len(row):index=0
  if not applied and index==len(row)-1 and (counter==row[index]['delay'] or row[index]['delay']==0):
   applied=True;row=p['final'];index=0;counter=0
   if not row:return None
 return row[index] if row else None
def behind(poly,point):
 x,y=point
 if x<poly[0][0]:return y<poly[0][1]
 if x>poly[-1][0]:return y<poly[-1][1]
 for a,z in zip(poly,poly[1:]):
  if z[0]>=x:return (z[0]-a[0])*(y-a[1])-(z[1]-a[1])*(x-a[0])<0
 raise AssertionError()
def order(elements):
 plain=sorted([e for e in elements if not e['polyline']],key=lambda e:(e['display_order'],e['creation_order']))
 animated=sorted([e for e in elements if e['polyline']],key=lambda e:min(p[1] for p in e['polyline']))
 out=[]
 for e in animated:
  retained=[]
  for p in plain:
   if behind(e['polyline'],p['sort_position']):out.append(p)
   else:retained.append(p)
  plain=retained;out.append(e)
 return out+plain
def paint(dst,f,pos,origin):
 if f is None:return
 src=image(f);x=math.floor(pos[0]+f['offset'][0]-origin[0]);y=math.floor(pos[1]+f['offset'][1]-origin[1]);h,w=src.shape[:2];H,W=dst.shape[:2]
 x0=max(x,0);y0=max(y,0);x1=min(x+w,W);y1=min(y+h,H)
 if x1<=x0 or y1<=y0:return
 s=src[y0-y:y1-y,x0-x:x1-x];d=dst[y0:y1,x0:x1];opaque=s[:,:,3]==255;d[opaque]=s[opaque];partial=(s[:,:,3]>0)&~opaque
 if partial.any():
  a=s[partial,3].astype(float);remaining=d[partial,3]*(255-a)/255;combined=a+remaining;d[partial,:3]=np.floor((s[partial,:3]*a[:,None]+d[partial,:3]*remaining[:,None])/combined[:,None]+.5);d[partial,3]=np.floor(combined+.5)
records=[]
for r in manifest['records']:
 if r['status']!='SOURCE_BINDINGS_PASS':continue
 c=json.loads((b/r['contract']).read_text());n=c['native'];selected=next(p for p in n['patch_states'] if p['id']==c['focus_patch_id']);terminal=r['terminal_tick'];samples=[]
 for tick in [-1,*range(terminal+3)]:
  dst=image(n['background']).copy();elements=[]
  if selected['integrate_in_background'] and tick>=terminal:paint(dst,selected['transition'][-1],selected['display_position'],n['origin'])
  for e in n['elements']:
   f=e.get('initial_frame') if not e['active'] else frame(e['frames'],max(0,tick),e['loop'])
   if e['active'] or f:elements.append({**e,'frame':f})
  for p in n['patch_states']:
   f=phase_frame(p,tick) if p['id']==selected['id'] else frame(p['initial'],max(0,tick),p['initial_loop'])
   if f is None:continue
   if p['layer']=='background':paint(dst,f,p['display_position'],n['origin'])
   else:elements.append({**p,'frame':f})
  for e in order(elements):paint(dst,e['frame'],e['display_position'],n['origin'])
  samples.append({'tick':tick,'rgba_sha256':hashlib.sha256(dst.tobytes()).hexdigest()})
 records.append({'id':r['id'],'contract':r['contract'],'cases':samples});print(r['id'],len(samples),flush=True)
(b/'independent-reference.json').write_text(json.dumps({'scope':'Independent frame-counter and source-order composition; every integer focus tick and two postcompletion ticks; surrounding ambient initial clocks follow global tick. No actor simulation.','records':records},indent=2)+'\n')
