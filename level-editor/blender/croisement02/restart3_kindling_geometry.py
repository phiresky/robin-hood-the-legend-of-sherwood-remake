"""Complete the source-observed kindling height with closed sticks and one binding."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.optimize import minimize
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';BOX=(130,940,198,1020);SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def construct(parameters,fit):
 points=np.asarray(fit['vertices']).copy();faces=[tuple(f) for f in fit['faces']]
 # Separate closed sticks retain grounded ends. The central taller sticks are
 # observed in silhouette; back arrangement and circular cross sections inferred.
 for j in range(28):
  ids=slice(j*20,j*20+20);p=points[ids];lower=p[:10].mean(axis=0);upper=p[10:].mean(axis=0)
  p[:10]=lower+(p[:10]-lower)*parameters[0];p[:10,2]-=p[:10,2].min()+.1
  p[10:]=upper+(p[10:]-upper)*parameters[1];p[10:,2]+=parameters[2]
  if j==27:p[10:,2]+=parameters[3]
  if j==23:p[10:,2]+=parameters[4]
  points[ids]=p
 # A single thin wrapping follows the source's dark cross-band; hidden return inferred.
 xr,yr,h,taper,cx,cy=fit['fitted'];z=25.;t=z/h;rx=xr*(1-t+t*taper)+1.3;ry=yr*(1-t+t*taper)+1.3
 center=np.array((cx-1.8*t,-cy/SIN+3*t,z));verts=points.tolist();start=len(verts);segments=48;sides=6
 for i in range(segments):
  a=i*math.tau/segments;p=center+np.array((rx*math.cos(a),ry*math.sin(a),.45*math.sin(a)))
  radial=np.array((math.cos(a),math.sin(a),0.))
  for k in range(sides):verts.append((p+.65*(radial*math.cos(k*math.tau/sides)+np.array((0,0,1))*math.sin(k*math.tau/sides))).tolist())
 for i in range(segments):
  for k in range(sides):faces.append((start+i*sides+k,start+((i+1)%segments)*sides+k,start+((i+1)%segments)*sides+(k+1)%sides,start+i*sides+(k+1)%sides))
 return np.asarray(verts),faces

def main():
 root=OUT/'restart3-kindling/outline-v1';root.mkdir(parents=True,exist_ok=False);fit=json.loads((OUT/'restart2-vegetation/kindling-outline-research/fit.json').read_text());mask=np.asarray(Image.open(OUT/'baseline/masks/000104.png').convert('L'))>0;target=np.zeros((80,68),bool);target[18:18+mask.shape[0],18:18+mask.shape[1]]=mask
 def raster(p):
  verts,faces=construct(p,fit);xy=np.column_stack((verts[:,0]-130,-verts[:,1]*SIN-verts[:,2]*COS-940));im=Image.new('L',(204,240));draw=ImageDraw.Draw(im)
  for f in faces:draw.polygon([tuple(xy[i]*3) for i in f],fill=255)
  return np.asarray(im)[1::3,1::3]>0
 def score(p):
  m=raster(p);return int((target&~m).sum())+1.2*int((m&~target).sum())
 result=minimize(score,[1.2,1.1,0,10,4],method='Powell',bounds=[(1,1.45),(.9,1.3),(-2,3),(7,14),(0,8)],options={'maxiter':5,'maxfev':600,'xtol':.06});p=result.x;m=raster(p);v,f=construct(p,fit);source=Image.open(OUT/'animation-references/composite-frame-0.png').crop(BOX).convert('RGBA');ov=np.array(source);ov[target&~m,:3]=(255,0,120);ov[m&~target,:3]=(0,170,255);Image.fromarray(ov).resize((340,400),Image.Resampling.NEAREST).save(root/'source-residual.png');source.resize((340,400),Image.Resampling.NEAREST).save(root/'source.png')
 report=dict(parameters=p.tolist(),vertices=v.tolist(),faces=f,stick_count=28,binding_count=1,source_box=BOX,inside=int((m&target).sum()),missing=int((target&~m).sum()),extra=int((m&~target).sum()),ground_minimum=float(v[:,2].min()),top=float(v[:,2].max()),limitations=['Depth, hidden stick arrangement and rope return inferred.','Native mask is preserved; silhouette fit does not establish every mask pixel as bark.','Each stick is a closed volume; binding intersects its outer surfaces.'])
 (root/'geometry.json').write_text(json.dumps(report,indent=2)+'\n');print({k:val for k,val in report.items() if k not in ('vertices','faces')})
if __name__=='__main__':main()
