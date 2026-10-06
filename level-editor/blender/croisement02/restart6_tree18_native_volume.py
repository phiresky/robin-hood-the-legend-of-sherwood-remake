"""Continuous inferred lower volume constrained by this tree's native radii."""
import json
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from skimage.measure import marching_cubes
ROOT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
out=ROOT/'restart6-source-coverage/tree18-native-volume-v1';out.mkdir(exist_ok=False)
sin=np.sin(np.deg2rad(35));cos=np.cos(np.deg2rad(35));ground=224.90138
paths=next(r['paths']for r in json.load(open(ROOT/'wood-traces.json'))if r['mask']==18);nodes=[]
for path in paths:
 for a,b in zip(path,path[1:]):
  for t in np.linspace(0,1,max(2,int(np.linalg.norm(np.array(a[:2])-b[:2])/1.)+1)):
   x,y,r=np.array(a)*(1-t)+np.array(b)*t;r=max(1.25,r*1.01+.35);z=max(r*.65,(ground-y)/cos)
   if z>126:continue
   nodes.append([x,-(y+z*cos)/sin,z,r])
nodes=np.array(nodes);spacing=.5;lo=np.floor((nodes[:,:3]-nodes[:,3,None]).min(0)-6);hi=np.ceil((nodes[:,:3]+nodes[:,3,None]).max(0)+6);shape=np.ceil((hi-lo)/spacing).astype(int)+1;field=np.full(tuple(shape),30.,np.float32)
for p in nodes:
 radius=p[3];a=np.maximum(0,np.floor((p[:3]-radius-5-lo)/spacing).astype(int));b=np.minimum(shape,np.ceil((p[:3]+radius+5-lo)/spacing).astype(int)+1);grid=np.ogrid[a[0]:b[0],a[1]:b[1],a[2]:b[2]];distance=np.sqrt(sum((grid[d]*spacing+lo[d]-p[d])**2 for d in range(3)))-radius;sl=tuple(slice(a[d],b[d])for d in range(3));field[sl]=np.minimum(field[sl],distance)
field=gaussian_filter(field,sigma=1.1/spacing);z=np.arange(shape[2])*spacing+lo[2];field=np.maximum(field,(.15-z)[None,None,:]);vertices,faces,_,_=marching_cubes(field,0,spacing=(spacing,)*3,gradient_direction='ascent');vertices+=lo;np.savez_compressed(out/'lower.npz',vertices=vertices,faces=faces);(out/'evidence.json').write_text(json.dumps(dict(native_mask=18,native_ground_y=ground,nodes=nodes.tolist(),spacing=spacing,smoothing=1.1,vertices=len(vertices),faces=len(faces),scope='Continuous own-source lower trunk/root radii. Hidden depth and per-root groundward ray adjustment inferred. No other tree reference used.',limitations=['Upper graft and full native source coverage pending.','Ground plane clipped atZ.15; no source ownership transfer.']),indent=2)+'\n')
