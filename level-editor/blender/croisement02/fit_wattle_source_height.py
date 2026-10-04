"""Read-only source silhouette fit of wattle height; do not alter approved geometry."""
import json,math,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.spatial import ConvexHull
from scipy.optimize import minimize
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from evidence_io import sha,write_json
OUT=ROOT/'level-editor/work/croisement02-refinement';SIN,COS=math.sin(math.radians(35)),math.cos(math.radians(35))

def main():
 base=OUT/'mixed-wood-audit/boundary-roles76-93-v1';members=json.loads((base/'post-geometry.json').read_text());dst=base/'height-fit-v1';dst.mkdir(exist_ok=False);crop=(526,741,792,1026);left,top,right,bottom=crop;size=(right-left,bottom-top)
 expected=np.zeros((1152,1792),bool);mask=np.asarray(Image.open(OUT/'baseline/masks/000099.png').convert('L'))>0;expected[750:1021,530:785]=mask;expected=expected[top:bottom,left:right]
 pieces=[np.asarray(r['vertices'])for r in members['members']]
 def raster(p):
  image=Image.new('L',size);dr=ImageDraw.Draw(image)
  for xyz in pieces:
   points=np.column_stack((xyz[:,0]+p[1]-left,-SIN*xyz[:,1]-COS*xyz[:,2]*p[0]+p[2]-top));hull=ConvexHull(points);dr.polygon([tuple(v)for v in points[hull.vertices]],fill=255)
  return np.asarray(image)>0
 def loss(p):
  a=raster(p);return 1-(a&expected).sum()/(a|expected).sum()
 result=minimize(loss,[1.5,0,0],bounds=[(.8,2.5),(-5,5),(-8,8)],method='Powell',options={'maxiter':10,'xtol':.02});rows=[];sheet=Image.new('RGB',(size[0]*2*3,size[1]*3))
 source=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop(crop))
 for i,(label,p)in enumerate([('baseline',[1,0,0]),('height fit',result.x)]):
  a=raster(p);overlay=source.copy();overlay[expected&~a]=[255,40,40];overlay[a&~expected]=[40,100,255];sheet.paste(Image.fromarray(overlay).resize((size[0]*3,size[1]*3),Image.Resampling.NEAREST),(i*size[0]*3,0));rows.append(dict(label=label,height_factor=float(p[0]),source_x_shift=float(p[1]),source_y_shift=float(p[2]),iou=1-loss(p),missing=int((expected&~a).sum()),extra=int((a&~expected).sum()),expected=int(expected.sum())))
 sheet.save(dst/'source-comparison.png');write_json(dst/'report.json',dict(status='Private read-only fitting hypothesis; source artwork and physical member review required',model_sha256=members['model_sha256'],mask_sha256=sha(OUT/'baseline/masks/000099.png'),records=rows,limitations=['Native99 is gameplay occupancy, not final object ownership.','Foreground76 must remain distinct and may visually cover physical fence.','No model or texture modified; silhouette metric alone does not approve reconstruction.']))
 print(rows)

if __name__=='__main__':main()
