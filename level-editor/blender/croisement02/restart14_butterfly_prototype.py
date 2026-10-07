"""Build one source-fitted physical butterfly hypothesis; no library writes."""
from pathlib import Path
import json,hashlib,math
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';OUT=BASE/'prototype-v1';OUT.mkdir(exist_ok=True)
report=json.loads((BASE/'source-audit-v1/report.json').read_text());seq=next(s for s in report['sequences'] if s['index']==8)
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));R=np.array([0,-COS,SIN]);DOWN=np.array([0,-SIN,-COS]);X=np.array([1,0,0]);frames=[]
for f in seq['frames']:
 a=np.array(Image.open(f['source']).convert('RGBA'));mask=a[:,:,3]>0;ys,xs=np.nonzero(mask);h,w=mask.shape;cx=float(xs.mean()+.5);cy=float(ys.mean()+.5)
 # The body orientation and hidden fold depth are conservative hypotheses.
 phase=f['index']/99;angle=.6*math.sin(phase*math.tau);axis=np.array([math.sin(angle),math.cos(angle)]);side=np.array([axis[1],-axis[0]])
 points=np.column_stack((xs+.5-cx,ys+.5-cy));span=max(1,float(np.max(abs(points@side))));fold=max(.30,min(2.2,5.2/span-.15));length=max(2,float(np.max(abs(points@axis))))
 def depth(x,y,back):
  p=np.array([x-cx,y-cy]);s=float(p@side);t=float(p@axis);body=math.exp(-(s/.65)**2)*max(.1,1-(t/(length+1))**2)
  camdepth=-abs(s)*fold + .30*math.sin(min(1,abs(s)/span)*math.pi) + .45*body
  thickness=.085+.78*body
  return camdepth-(thickness if back else 0)
 vertices=[];faces=[];uv=[];cache={}
 def vertex(x,y,back):
  key=(x,y,back)
  if key not in cache:
   cache[key]=len(vertices);vertices.append(((x-cx)*X+(y-cy)*DOWN+depth(x,y,back)*R).tolist())
  return cache[key]
 def face(coords):
  faces.append([vertex(*v) for v in coords]);uv.append([[v[0]/w,1-v[1]/h] for v in coords])
 for y,x in zip(ys.tolist(),xs.tolist()):
  q=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)]
  face([(xx,yy,False) for xx,yy in reversed(q)]);face([(xx,yy,True) for xx,yy in q])
  for k,(dx,dy) in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
   xx,yy=x+dx,y+dy
   if 0<=xx<w and 0<=yy<h and mask[yy,xx]:continue
   p0=q[k];p1=q[(k+1)%4];face([(*p0,False),(*p1,False),(*p1,True),(*p0,True)])
 va=np.array(vertices);projection=np.column_stack((va@X,va@DOWN));expected=np.array([[key[0]-cx,key[1]-cy] for key,idx in sorted(cache.items(),key=lambda z:z[1])]);err=float(abs(projection-expected).max());assert err<1e-10
 # Provisional absolute altitude is explicitly not measured from the artwork.
 sx=f['bbox'][0]+cx;sy=f['bbox'][1]+cy;altitude=60.0;world=[sx,-(sy+COS*altitude)/SIN,altitude]
 frames.append({**f,'vertices':vertices,'faces':faces,'uv':uv,'centroid_local':[cx,cy],'provisional_world_anchor':world,'projected_anchor':[sx,sy],'max_projection_error':err,'depth_span':float(np.ptp(va@R)),'body_axis_radians':angle,'wing_fold_rate':fold,'inferred_altitude':altitude})
packet={'status':'PRIVATE_REPRESENTATIVE_PROTOTYPE','sequence':8,'profile':seq['profile'],'source_audit_sha256':hashlib.sha256((BASE/'source-audit-v1/report.json').read_bytes()).hexdigest(),'cycle_ticks':198,'frames':frames,'hypothesis':{'structure':'Closed thin curved wing membranes joined through a thicker central body band. Source pixel boundary is retained; no camera-facing sprite or automatic billboard rotation.','backs':'Own front RGB continued onto hidden reverse surfaces; inferred, not observed reverse pattern.','trajectory':'Source projected frame anchors exact; constant world Z60 is a provisional depth choice, NOT sprite elevation or a measured flight altitude. Receiver/foliage occlusion needs separate placement review.','pose':'Fold depth and body orientation inferred from a modest continuous parameterization; source-facing shape/colors remain phase-specific. Geometry swaps exactly at native two-tick boundaries; no invented source interpolation.','scope':'Only papillon01 physical prototype; other six remain source audit only.'}}
(OUT/'packet.json').write_text(json.dumps(packet,separators=(',',':')))
# Native path evidence: positions are observed; the depth above is not.
canvas=Image.new('RGB',(660,420),'#252525');d=ImageDraw.Draw(canvas);lo=np.min([f['projected_anchor'] for f in frames],axis=0);hi=np.max([f['projected_anchor'] for f in frames],axis=0)
pts=[tuple((np.array(f['projected_anchor'])-lo)*4+45) for f in frames];d.line(pts,fill='#b7b7b7',width=2)
for i in [0,2,8,18,29,37,53,75,84,98]:
 f=frames[i];im=Image.open(f['source']).convert('RGBA').resize((f['bbox'][2]*3,f['bbox'][3]*3),Image.Resampling.NEAREST);x,y=pts[i];canvas.paste(im,(round(x)-im.width//2,round(y)-im.height//2),im);d.text((x+10,y+12),str(i),fill='white')
d.text((10,395),'Observed screen path only; hidden depth/altitude inferred.',fill='white');canvas.save(OUT/'native-trajectory.png')
summary={'frames':99,'cycle_ticks':198,'native_projection_error_max':max(f['max_projection_error'] for f in frames),'vertex_count':sum(len(f['vertices']) for f in frames),'max_depth_span':max(f['depth_span'] for f in frames),'min_depth_span':min(f['depth_span'] for f in frames),'packet_sha256':hashlib.sha256((OUT/'packet.json').read_bytes()).hexdigest()};(OUT/'construction.json').write_text(json.dumps(summary,indent=2)+'\n');print(summary)
