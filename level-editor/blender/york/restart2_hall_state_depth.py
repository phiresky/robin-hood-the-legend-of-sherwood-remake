"""Inspect native character-mask thresholds for the four castle cover combinations."""
import hashlib
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
DEST=OUT/'restart2/hall-cover-depth-combinations-v1'
if DEST.exists():
    raise FileExistsError(DEST)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from masks_to_depth import character_threshold, initial_mask_indices

level_path=OUT/'baseline/york.rhp.json'
level=json.loads(level_path.read_text())
source_dir=OUT/'restart2/hall-cover-source-combinations-v1'
states=json.loads((source_dir/'manifest.json').read_text())['states']
inventory_path=OUT/'baseline/masks/manifest.json'
inventory=json.loads(inventory_path.read_text())
entries={r['index']:r for r in inventory['masks']}
initial=initial_mask_indices(level)
crop=(2720,440,3055,800)
left,top,right,bottom=crop
width,height=right-left,bottom-top
map_height=Image.open(OUT/'baseline/covered.png').height
DEST.mkdir(parents=True)
sheet=Image.new('RGB',(width*4,(height+25)*2),'#101820')
rows=[]
for column,state in enumerate(states):
    active=initial-set(state['removed_native_masks'])
    result=np.zeros((height,width),dtype=np.uint16)
    labels=[]
    for index in sorted(active):
        mask=level['masks'][index]
        if mask['layer']!=4 or not mask['mask_type']&1:
            continue
        x,y=mask['box_top_left'];w,h=mask['box_size']
        x0,y0,x1,y1=max(left,x),max(top,y),min(right,x+w),min(bottom,y+h)
        if x0>=x1 or y0>=y1:
            continue
        path=inventory_path.parent/entries[index]['png']
        bits=np.asarray(Image.open(path).convert('L'))[y0-y:y1-y,x0-x:x1-x]!=0
        if not bits.any():
            continue
        thresholds=character_threshold(mask['character_polyline'],np.arange(x0,x1)+.5)
        encoded=np.rint(np.clip(thresholds/map_height,0,1)*65535).astype(np.uint16)
        region=result[y0-top:y1-top,x0-left:x1-left]
        np.maximum(region,np.where(bits,encoded[None,:],0),out=region)
        yy,xx=np.nonzero(bits)
        n=np.argmin((yy-yy.mean())**2+(xx-xx.mean())**2)
        labels.append((index,x0-left+int(xx[n]),y0-top+int(yy[n])))
    name=Path(state['image']).stem
    field=DEST/f'{name}.depth16.png'
    Image.fromarray(result).save(field)
    preview=Image.fromarray((result//257).astype(np.uint8)).convert('RGB')
    draw=ImageDraw.Draw(preview)
    for index,x,y in labels:
        draw.text((x,y),str(index),fill='#ffff00',stroke_width=1,stroke_fill='black')
    preview.save(DEST/f'{name}.mask-ids.png')
    source=Image.open(source_dir/state['image']).convert('RGB').crop(crop)
    for row,image in enumerate((source,preview)):
        ox,oy=column*width,row*(height+25)
        sheet.paste(image,(ox,oy+25))
        title=('Native source' if row==0 else 'Native layer 4 mask IDs')+f' | combination {column}'
        ImageDraw.Draw(sheet).text((ox+5,oy+8),title,fill='white')
    rows.append({'source':state['image'],'removed_masks':state['removed_native_masks'],
                 'included_masks':[n for n,x,y in labels],'depth_sha256':hashlib.sha256(field.read_bytes()).hexdigest()})
sheet.save(DEST/'native-source-and-depth.png')
(DEST/'manifest.json').write_text(json.dumps({
    'scope':'Native source and approximate character occlusion thresholds, not physical surface depth or geometry approval.',
    'crop':crop,'layer':4,'level_sha256':hashlib.sha256(level_path.read_bytes()).hexdigest(),
    'source_combinations_sha256':hashlib.sha256((source_dir/'manifest.json').read_bytes()).hexdigest(),
    'encoding':'round(clamp(character_polyline_y / full_map_height,0,1)*65535); zero uncovered, overlapping thresholds use maximum.',
    'states':rows,
    'limitations':['Thresholds sample sprite-pixel X rather than actor-anchor X.','Projectile thresholds are separate.','Mask availability follows the two recorded patch removals; mission reachability is not asserted.']},indent=2)+'\n')
print(json.dumps({'states':len(rows),'crop':crop,'layer':4}))
