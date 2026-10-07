"""Add a bounded physical body inside the observed butterfly source silhouette."""
from pathlib import Path
import json,math,hashlib
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies';OUT=BASE/'prototype-v2';OUT.mkdir(exist_ok=True)
p=json.loads((BASE/'prototype-v1/packet.json').read_text());sin=math.sin(math.radians(35));cos=math.cos(math.radians(35));R=np.array([0,-cos,sin]);D=np.array([0,-sin,-cos]);X=np.array([1,0,0]);proof=[]
for f in p['frames']:
 a=np.array(Image.open(f['source']));m=a[:,:,3]>0;h,w=m.shape;cx,cy=f['centroid_local'];axis=np.array([math.sin(f['body_axis_radians']),math.cos(f['body_axis_radians'])]);side=np.array([axis[1],-axis[0]])
 samples=np.array([(u,v) for u in np.linspace(-1,1,25) for v in np.linspace(-1,1,17) if u*u+v*v<=1.00001]);best=None
 for yy,xx in np.column_stack(np.nonzero(m)):
  center=np.array([xx+.5,yy+.5]);distance=float(np.linalg.norm(center-[cx,cy]))
  if distance>3.5:continue
  for length in np.linspace(3.5,.45,32):
   xy=center+samples[:,0,None]*length*axis+samples[:,1,None]*.30*side;ix=np.floor(xy[:,0]).astype(int);iy=np.floor(xy[:,1]).astype(int)
   if np.any(ix<0) or np.any(ix>=w) or np.any(iy<0) or np.any(iy>=h) or not m[iy,ix].all():continue
   score=length-.55*distance-.001*float(a[yy,xx,:3].mean())
   if best is None or score>best[0]:best=(score,center,length)
   break
 assert best;_,center,length=best;start=len(f['vertices']);verts=[];uv=[]
 def add(t,s,depth):
  xy=center+axis*t+side*s;pos=(xy[0]-cx)*X+(xy[1]-cy)*D+depth*R;verts.append(pos.tolist());uv.append([float(xy[0]/w),float(1-xy[1]/h)])
 add(length,0,.45)
 for ring in range(1,8):
  phi=math.pi*ring/8
  for j in range(16):
   theta=math.tau*j/16;add(length*math.cos(phi),.30*math.sin(phi)*math.cos(theta),.45+.55*math.sin(phi)*math.sin(theta))
 add(-length,0,.45);polys=[]
 for j in range(16):polys.append([0,1+j,1+(j+1)%16])
 for ring in range(6):
  for j in range(16):a0=1+ring*16+j;a1=1+ring*16+(j+1)%16;polys.append([a0,a0+16,a1+16,a1])
 end=len(verts)-1
 for j in range(16):polys.append([end,1+6*16+(j+1)%16,1+6*16+j])
 f['vertices']+=verts;f['faces'] += [[start+i for i in poly] for poly in polys];f['uv'] += [[uv[i] for i in poly] for poly in polys]
 f['physical_body']={'source_center':center.tolist(),'source_half_length':float(length),'source_radius':.30,'camera_depth_radius':.55,'source_projection_inside_known_alpha':True}
 proof.append({'phase':f['index'],**f['physical_body']})
p['hypothesis']['structure']='Closed thin curved wing membranes plus a distinct elongated body volume fitted wholly inside observed opaque source. Pixel boundary is retained; camera-facing billboard rotation is absent.'
p['hypothesis']['body']='Small elongated body has inferred thickness; its native projected UV/RGB derives from the exact same source image. No invented antennas or unseen pattern.'
(OUT/'packet.json').write_text(json.dumps(p,separators=(',',':')));(OUT/'body-guard.json').write_text(json.dumps({'frames':proof,'known_positive_rgba_unchanged':True,'source':str(BASE/'prototype-v1/packet.json'),'packet_sha256':hashlib.sha256((OUT/'packet.json').read_bytes()).hexdigest()},indent=2)+'\n');print('BODY_READY',len(proof),min(r['source_half_length'] for r in proof),max(r['source_half_length'] for r in proof))
