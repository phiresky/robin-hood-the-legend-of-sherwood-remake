"""True continuous union of own native lower root centerlines."""
import json,sys
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from skimage.measure import marching_cubes
OUT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement';ROOT=OUT/'restart6-source-coverage';number=int(sys.argv[1]);out=ROOT/f'tree{number}-toe-volume-v1';out.mkdir(exist_ok=False);paths=next(r['paths']for r in json.load(open(OUT/'wood-traces.json'))if r['mask']==number);sin=np.sin(np.deg2rad(35));cos=np.cos(np.deg2rad(35));ground=150 if number==19 else 956.4;limit=50 if number==19 else 78
if number==19:paths[-1]=paths[-1]+[[1605.5,154.0,3.0]]
else:paths[-3]=paths[-3]+[[109.0,945.0,3.5]]
nodes=[]
for path in paths:
 for a,b in zip(path,path[1:]):
  for t in np.linspace(0,1,max(2,int(np.linalg.norm(np.array(a[:2])-b[:2]))+1)):
   x,y,r=np.array(a)*(1-t)+np.array(b)*t;r=max(1.1,r);z=max(r*.55,(ground-y)/cos)
   if z-r>limit+7:continue
   nodes.append([x,-(y+z*cos)/sin,z,r])
nodes=np.array(nodes);spacing=.45 if number==19 else .65;lo=np.floor((nodes[:,:3]-nodes[:,3,None]).min(0)-4);hi=np.ceil((nodes[:,:3]+nodes[:,3,None]).max(0)+4);shape=np.ceil((hi-lo)/spacing).astype(int)+1;field=np.full(tuple(shape),30.,np.float32)
for p in nodes:
 radius=p[3];a=np.maximum(0,np.floor((p[:3]-radius-4-lo)/spacing).astype(int));b=np.minimum(shape,np.ceil((p[:3]+radius+4-lo)/spacing).astype(int)+1);grid=np.ogrid[a[0]:b[0],a[1]:b[1],a[2]:b[2]];distance=np.sqrt(sum((grid[d]*spacing+lo[d]-p[d])**2 for d in range(3)))-radius;sl=tuple(slice(a[d],b[d])for d in range(3));field[sl]=np.minimum(field[sl],distance)
field=gaussian_filter(field,sigma=.75/spacing);z=np.arange(shape[2])*spacing+lo[2];field=np.maximum(field,(.15-z)[None,None,:]);v,f,_,_=marching_cubes(field,0,spacing=(spacing,)*3,gradient_direction='ascent');v+=lo;np.savez_compressed(out/'lower.npz',vertices=v,faces=f);(out/'construction.json').write_text(json.dumps(dict(native_mask=number,native_ground_reference=ground,nodes=nodes.tolist(),spacing=spacing,smoothing=.75,vertices=len(v),faces=len(f),scope='Own native centerline/radius union with bounded lower toe continuation. Groundward source-ray depth is inferred. No other tree reference; no material change.'),indent=2)+'\n')
