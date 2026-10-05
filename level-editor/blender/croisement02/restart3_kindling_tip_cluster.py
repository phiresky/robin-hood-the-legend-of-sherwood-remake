"""Source-supported staggered top sticks instead of one isolated tall tip."""
import json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement';src=OUT/'restart3-kindling/outline-v1/geometry.json';dest=OUT/'restart3-kindling/outline-v2';dest.mkdir(exist_ok=False)
d=json.loads(src.read_text());v=np.array(d['vertices']);changes=[]
for j,lift in [(24,7.),(25,11.),(26,8.)]:
 v[j*20+10:j*20+20,2]+=lift;changes.append(dict(stick=j,top_height_added=lift))
d['vertices']=v.tolist();d['tip_refinement']=changes;d['parent_geometry']=str(src)
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));xy=np.column_stack((v[:,0]-130,-v[:,1]*s-v[:,2]*c-940));im=Image.new('L',(204,240));draw=ImageDraw.Draw(im)
for f in d['faces']:draw.polygon([tuple(xy[i]*3) for i in f],fill=255)
m=np.array(im)[1::3,1::3]>0;target=np.zeros((80,68),bool);mask=np.array(Image.open(OUT/'baseline/masks/000104.png').convert('L'))>0;target[18:18+mask.shape[0],18:18+mask.shape[1]]=mask
source=Image.open(OUT/'animation-references/composite-frame-0.png').crop((130,940,198,1020)).convert('RGBA');ov=np.array(source);ov[target&~m,:3]=(255,0,120);ov[m&~target,:3]=(0,170,255);Image.fromarray(ov).resize((340,400),Image.Resampling.NEAREST).save(dest/'source-residual.png');d.update(inside=int((m&target).sum()),missing=int((target&~m).sum()),extra=int((m&~target).sum()));(dest/'geometry.json').write_text(json.dumps(d,indent=2)+'\n');print({k:d[k] for k in ['inside','missing','extra']})
