"""Read-only native art, mask and occlusion-threshold context for residual roles."""
import json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import label
REPO=Path(__file__).resolve().parents[2];OUT=REPO/'work/croisement02-refinement';ROOT=OUT/'restart6-source-coverage';DEST=ROOT/'source-role132-v1';DEST.mkdir(exist_ok=False)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB');inventory=json.loads((OUT/'review-mask-inventory.json').read_text())['masks'];residual=json.loads((ROOT/'remaining-inventory-v1/report.json').read_text())['rows'];depth_paths=[REPO/f'work/map-compile/occlusion-depth-all-layers/croisement02.layer-{i}.occlusion-depth.png'for i in range(2)];depths=[np.array(Image.open(p))for p in depth_paths];colors=[(255,70,80),(70,200,255),(255,210,50),(230,80,255),(80,255,130),(255,140,50),(160,150,255)];rows=[]
for number,box in[(19,(1574,117,1630,180)),(25,(82,849,160,975)),(95,(1507,747,1548,814))]:
 item=next(r for r in inventory if r['index']==number);coords=next(r['coordinates']for r in residual if r['mask']==number);mask=np.zeros((native.height,native.width),bool)
 for x,y in coords:mask[y,x]=True
 labels,count=label(mask,np.ones((3,3)));original=native.crop(box);marked=original.copy();draw=ImageDraw.Draw(marked);components=[];own=np.zeros((native.height,native.width),np.uint8);ox,oy=item['box_top_left'];mw,mh=item['box_size'];own[oy:oy+mh,ox:ox+mw]=np.array(Image.open(item['png']).convert('L'))
 for i in range(1,count+1):
  yy,xx=np.where(labels==i);color=colors[(i-1)%len(colors)]
  for x,y in zip(xx,yy):draw.point((int(x-box[0]),int(y-box[1])),fill=color)
  components.append(dict(component=i,count=len(xx),coordinates=[[int(x),int(y)]for x,y in zip(xx,yy)],bbox=[int(xx.min()),int(yy.min()),int(xx.max()+1),int(yy.max()+1)],source_rgb=[list(native.getpixel((int(x),int(y))))for x,y in zip(xx,yy)],mask_owned=int(np.count_nonzero(own[yy,xx])),threshold_values=[sorted(set(int(v)for v in a[yy,xx]))for a in depths],color=color))
 original.save(DEST/f'{number}-source.png');marked.save(DEST/f'{number}-marked.png');maskimage=Image.fromarray(own).crop(box).convert('RGB');panels=[('Original native RGB',original),('Exposed center components',marked),('Original own-mask coverage',maskimage)]
 for i,a in enumerate(depths):
  crop=a[box[1]:box[3],box[0]:box[2]];gray=np.rint(crop.astype(float)/65535*255).astype('uint8');panels.append((f'Layer{i} threshold, not physical depth',Image.fromarray(gray).convert('RGB')))
 scale=5;pw=original.width*scale;ph=original.height*scale;sheet=Image.new('RGB',(pw*len(panels),ph+45),(25,25,25));d=ImageDraw.Draw(sheet)
 for j,(title,pic)in enumerate(panels):sheet.paste(pic.resize((pw,ph),Image.Resampling.NEAREST),(j*pw,45));d.text((j*pw+4,5),title,fill='white');d.text((j*pw+4,20),f'Mask{number}; source box{box}',fill='white')
 sheet.save(DEST/f'{number}-context-sheet.png');rows.append(dict(mask=number,box=box,source_mask=item,components=components,exposed_count=len(coords),character_polyline=item['character_polyline'],projectile_polyline=item['projectile_polyline']))
manifest=dict(total=132,records=rows,source_files={str(p):sha(p)for p in[OUT/'animation-references/composite-frame-0.png',OUT/'review-mask-inventory.json',ROOT/'remaining-inventory-v1/report.json',*depth_paths]},limitations=['Mask ownership and occlusion thresholds do not establish wood versus foliage or physical depth.','All role classification remains explicit visual inference; no geometry, atlas or source-role transfer in this packet.'])
(DEST/'evidence.json').write_text(json.dumps(manifest,indent=2)+'\n');print(DEST)
