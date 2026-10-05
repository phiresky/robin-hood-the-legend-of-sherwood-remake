"""Constrain a closed two-part hay mound to its observed native silhouette."""
import argparse,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import gaussian_filter1d,label,binary_opening
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';DEST=OUT/'restart3-hay/outline-v1';BOX=(860,930,1050,1110);SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def raster(parts):
 image=Image.new('L',(570,540));draw=ImageDraw.Draw(image)
 for part in parts:
  v=np.array(part['vertices']);xy=np.column_stack((v[:,0]-860,-v[:,1]*SIN-v[:,2]*COS-930))
  for f in part['faces']:draw.polygon([tuple(xy[i]*3) for i in f],fill=255)
 return np.asarray(image)[1::3,1::3]>0

def radius(mask,origin):
 y,x=np.nonzero(mask);dx=x+.5+860-origin[0];dy=y+.5+930-origin[1];theta=np.mod(np.arctan2(dy,dx),math.tau);bins=np.floor(theta/math.tau*360).astype(int);r=np.zeros(360);np.maximum.at(r,bins,np.hypot(dx,dy));have=np.flatnonzero(r);r=np.interp(np.arange(360),np.r_[have-360,have,have+360],np.tile(r[have],3));return gaussian_filter1d(r,2.,mode='wrap')

def main():
 global DEST
 parser=argparse.ArgumentParser();parser.add_argument('--version',type=int,choices=[1,2],default=1);args=parser.parse_args();DEST=OUT/f'restart3-hay/outline-v{args.version}'
 DEST.mkdir(parents=True,exist_ok=False);source=OUT/'restart2-vegetation/hay-outline-research/application-plan.json';plan=json.loads(source.read_text());parts=plan['parts'];core=np.array(Image.open(OUT/'restart2-vegetation/hay-outline-research/core-domain.png').convert('L'))>0;origin=np.array([plan['parameters'][4],1019.]);history=[]
 yy,xx=np.ogrid[-3:4,-3:4];core_fit=binary_opening(core,structure=xx*xx+yy*yy<=9) if args.version==2 else core
 for iteration in range(3):
  body=raster(parts);old=radius(body,origin);wanted=radius(core_fit,origin);ratios=np.clip(wanted/old,.8,1.25);history.append(dict(inside=int((body&core).sum()),missing=int((~body&core).sum()),extra=int((body&~core).sum())))
  for part in parts:
   v=np.array(part['vertices']);xy=np.column_stack((v[:,0],-v[:,1]*SIN-v[:,2]*COS));d=xy-origin;r=np.linalg.norm(d,axis=1);angle=np.mod(np.arctan2(d[:,1],d[:,0]),math.tau)/math.tau*360;scale=np.interp(angle,np.arange(361),np.r_[ratios,ratios[0]]);extent=np.interp(angle,np.arange(361),np.r_[old,old[0]]);weight=np.clip(r/extent,0,1)**1.5;delta=d*((scale-1)*weight)[:,None];v[:,0]+=delta[:,0];v[:,1]-=delta[:,1]/SIN;part['vertices']=v.tolist()
 body=raster(parts);native=np.zeros_like(core);mask=np.array(Image.open(OUT/'baseline/masks/000124.png').convert('L'))>0;native[36:36+mask.shape[0],18:18+mask.shape[1]]=mask;residual=native&~body;source_image=Image.open(OUT/'animation-references/composite-frame-0.png').crop(BOX).convert('RGBA');overlay=np.array(source_image);overlay[residual,:3]=(255,0,120);overlay[body&~native,:3]=(0,170,255);Image.fromarray(overlay).resize((760,720),Image.Resampling.NEAREST).save(DEST/'source-residual.png');Image.fromarray(residual.astype('uint8')*255).save(DEST/'remaining-straw.png');source_image.resize((760,720),Image.Resampling.NEAREST).save(DEST/'source.png');plan.update(parent_plan=str(source),source_box=BOX,radial_origin=origin.tolist(),history=history,core_result=dict(inside=int((body&core).sum()),missing=int((core&~body).sum()),extra=int((body&~core).sum())),native_remaining=int(residual.sum()),status='Private closed rounded main mound; frayed native straw remains to construct',deformation='Bounded native screen-radius correction preserves every vertex height and ground contact. Hidden depth remains inferred.');(DEST/'geometry.json').write_text(json.dumps(plan,indent=2)+'\n');print({k:plan[k] for k in ['history','core_result','native_remaining']})
if __name__=='__main__':main()
