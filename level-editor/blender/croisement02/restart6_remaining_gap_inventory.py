"""Attribute remaining exposed centers to native masks without transferring ownership."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[2]/'work/croisement02-refinement'
out=ROOT/'restart6-source-coverage/remaining-inventory-v1';out.mkdir(exist_ok=False)
mask=np.array(Image.open(ROOT/'restart4-floor-closure-input-v1/native-exposed487.png'))>0
source=np.array(Image.open(ROOT/'animation-references/composite-frame-0.png').convert('RGB'));covered=np.zeros_like(mask);rows=[]
for m in json.load(open(ROOT/'review-mask-inventory.json'))['masks']:
 x,y=m['box_top_left'];w,h=m['box_size'];native=np.array(Image.open(m['png']))>0;local=native&mask[y:y+h,x:x+w];yy,xx=np.where(local)
 if not len(xx):continue
 covered[y:y+h,x:x+w]|=local;points=np.column_stack((xx+x,yy+y));box=[max(0,int(points[:,0].min())-14),max(0,int(points[:,1].min())-14),min(mask.shape[1],int(points[:,0].max())+15),min(mask.shape[0],int(points[:,1].max())+15)];a,b,c,d=box;orig=source[b:d,a:c].copy();highlight=orig.copy();highlight[mask[b:d,a:c]]=[255,0,140];size=(max(160,(c-a)*4),max(160,(d-b)*4));im=Image.new('RGB',(size[0]*2,size[1]+24),(30,30,30));im.paste(Image.fromarray(orig).resize(size,Image.Resampling.NEAREST),(0,24));im.paste(Image.fromarray(highlight).resize(size,Image.Resampling.NEAREST),(size[0],24));ImageDraw.Draw(im).text((6,6),f'Native mask {m["index"]}: {len(points)} exposed centers / original + highlighted',fill='white');path=out/f'mask-{m["index"]}.png';im.save(path);rows.append(dict(mask=m['index'],pixels=len(points),coordinates=points.tolist(),source_box=box,comparison=str(path)))
unknown=np.column_stack(np.where(mask&~covered)[::-1]);(out/'report.json').write_text(json.dumps(dict(total=int(mask.sum()),native_mask_attributed=int((covered&mask).sum()),outside_inventory_masks=unknown.tolist(),rows=sorted(rows,key=lambda r:-r['pixels']),scope='Read-only source-mask overlap. Counts do not prove physical wood/fence ownership, especially mottled fringes.'),indent=2)+'\n')
