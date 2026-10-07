"""Source-only branch centerline proposal; never infers physical depth or geometry."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from scipy.ndimage import distance_transform_edt,label
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];base=ROOT/'level-editor/work/croisement01-refinement';p=argparse.ArgumentParser();p.add_argument('mask',type=int);p.add_argument('output',type=Path);p.add_argument('--resume-incomplete',action='store_true');a=p.parse_args();assert not (a.output/'trace.json').exists();a.output.mkdir(exist_ok=a.resume_incomplete);row=next(r for r in json.loads((base/'baseline/masks/manifest.json').read_text())['masks'] if r['index']==a.mask);path=base/'baseline/masks'/row['png'];original=np.asarray(Image.open(path).convert('L'))>0;sk=original.copy();radius=distance_transform_edt(original)
for iteration in range(1000):
 removed=0
 for phase in range(2):
  q=np.pad(sk,1);h,w=sk.shape;neighbors=[q[1+dy:1+dy+h,1+dx:1+dx+w] for dy,dx in [(-1,0),(-1,1),(0,1),(1,1),(1,0),(1,-1),(0,-1),(-1,-1)]];b=sum(n.astype('uint8') for n in neighbors);transitions=sum((~x&y).astype('uint8') for x,y in zip(neighbors,neighbors[1:]+neighbors[:1]));n,e,s,wv=[neighbors[i] for i in [0,2,4,6]]
  constraint=~(n&e&s)&~(e&s&wv) if phase==0 else ~(n&e&wv)&~(n&s&wv);cut=sk&(b>=2)&(b<=6)&(transitions==1)&constraint;removed+=int(cut.sum());sk[cut]=False
 if not removed:break
assert iteration<999
coords={tuple(int(x) for x in v) for v in np.argwhere(sk)};adj={xy:[(xy[0]+dy,xy[1]+dx) for dy in [-1,0,1] for dx in [-1,0,1] if (dy or dx) and (xy[0]+dy,xy[1]+dx) in coords and not (dy and dx and ((xy[0]+dy,xy[1]) in coords or (xy[0],xy[1]+dx) in coords))] for xy in coords};nodes={xy for xy in coords if len(adj[xy])!=2};visited=set();paths=[]
for start in sorted(nodes):
 for nxt in adj[start]:
  edge=tuple(sorted([start,nxt]))
  if edge in visited:continue
  visited.add(edge);trace=[start,nxt];prev,current=start,nxt
  while current not in nodes:
   options=[xy for xy in adj[current] if xy!=prev];assert len(options)==1;following=options[0];edge=tuple(sorted([current,following]))
   if edge in visited:break
   visited.add(edge);trace.append(following);prev,current=current,following
  paths.append(trace)
x0,y0=row['box_top_left'];image=Image.open(base/'baseline/covered.png').convert('RGB').crop((x0,y0,x0+original.shape[1],y0+original.shape[0]));overlay=image.resize((image.width*2,image.height*2),Image.Resampling.NEAREST);draw=ImageDraw.Draw(overlay)
for trace in paths:
 if len(trace)>1:draw.line([(x*2+1,y*2+1) for y,x in trace],fill=(255,70,210),width=1)
for y,x in nodes:
 if len(adj[(y,x)])==1:draw.ellipse((x*2-2,y*2-2,x*2+3,y*2+3),outline=(70,255,110))
overlay.save(a.output/'source-centerline-proposal.png');Image.fromarray((sk*255).astype('uint8')).save(a.output/'skeleton.png');_,components=label(sk,np.ones((3,3)));data=dict(status='Unreviewed source tracing only; no physical geometry/depth or semantic owner claim',native_mask=a.mask,native_mask_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),iterations=iteration+1,skeleton_pixels=len(coords),connected_components=int(components),polyline_count=len(paths),untraced_edges=sum(len(v) for v in adj.values())//2-len(visited),source_offset=[x0,y0],polylines=[[[x+x0,y+y0,float(radius[y,x])] for y,x in trace] for trace in paths],limitations=['Short alpha islands, vines and antialias branches need semantic review','Pixel distance radius is projected width only; actual tube depth/branch arrangement requires independent inference','No disconnected source pieces joined automatically; all current gameplay and source assets unchanged']);(a.output/'trace.json').write_text(json.dumps(data,indent=2)+'\n');print({k:data[k] for k in ['skeleton_pixels','connected_components','polyline_count']})
