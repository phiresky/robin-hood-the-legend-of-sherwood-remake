"""Expose the complete disputed domain without assigning material by color."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import label, binary_erosion

ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/active/croisement01'
PRIOR=BASE/'restart2/tree14-layered-comparison-v1'
OUT=BASE/'restart2/tree14-complete-material-casebook-v1'
assert not OUT.exists()
OUT.mkdir()
source_path=BASE/'baseline/covered.png'
source=Image.open(source_path).convert('RGB')
source_rgba=source.convert('RGBA')
domain=np.asarray(Image.open(PRIOR/'ambiguous-restored.png').convert('L'))>0
components,count=label(domain,np.ones((3,3),dtype=bool))
areas=np.bincount(components.ravel())
order=sorted(range(1,count+1),key=lambda i:(-areas[i],i))
wood_hit=np.load(PRIOR/'receiver-depths.npz')['wood_hit']
rows=[]
for rank,component in enumerate(order,1):
    selected=components==component;y,x=np.nonzero(selected)
    box=(int(x.min()+645),int(y.min()+39),int(x.max()+646),int(y.max()+40))
    context=(max(645,box[0]-10),max(39,box[1]-10),min(985,box[2]+10),min(442,box[3]+10))
    plain=source.crop(context);w,h=plain.size;scale=5
    unmarked=plain.resize((w*scale,h*scale),Image.Resampling.NEAREST)
    marked=unmarked.copy();draw=ImageDraw.Draw(marked)
    boundary=selected&~binary_erosion(selected)
    for yy,xx in zip(*np.nonzero(boundary)):
        sx=(xx+645-context[0])*scale;sy=(yy+39-context[1])*scale
        draw.rectangle((sx,sy,sx+scale-1,sy+scale-1),outline='#e980dd',width=1)
    isolated=np.zeros((h,w,4),np.uint8);crop=np.asarray(source.crop(context));mask=selected[context[1]-39:context[3]-39,context[0]-645:context[2]-645]
    isolated[:,:,:3]=crop;isolated[:,:,3]=mask*255
    isolated_image=Image.fromarray(isolated).resize((w*scale,h*scale),Image.Resampling.NEAREST)
    board=Image.new('RGB',(w*scale*3,h*scale+45),'#292930');draw=ImageDraw.Draw(board)
    board.paste(unmarked,(0,45));board.paste(marked,(w*scale,45));board.paste(isolated_image,(w*scale*2,45),isolated_image)
    for i,text in enumerate(('Untouched source','Component boundary','Disputed source only')):draw.text((i*w*scale+4,24),text,fill='white')
    draw.text((4,5),f'Case {rank}: {len(x)} pixels | native bbox {box}',fill='white')
    filename=f'case-{rank:02}.png';board.save(OUT/filename)
    samples=[dict(pixel=[int(xx+645),int(yy+39)],rgba=list(source_rgba.getpixel((int(xx+645),int(yy+39)))),wood_hit=bool(wood_hit[yy,xx])) for yy,xx in zip(y,x)]
    rows.append(dict(case=rank,pixels=len(x),bbox=box,context_bbox=context,image=filename,wood_geometry_hits=int((selected&wood_hit).sum()),status='UNCLASSIFIED_NO_FILL',samples=samples))
assert sum(row['pixels'] for row in rows)==2443
report=dict(status='COMPLETE_DISPUTED_DOMAIN_CASEBOOK_NO_ASSIGNMENTS',source_sha256=hashlib.sha256(source_path.read_bytes()).hexdigest(),
    parent_mask_sha256=hashlib.sha256((PRIOR/'ambiguous-restored.png').read_bytes()).hexdigest(),total_pixels=2443,components=count,
    limitations=['Connected components are review units, not material classes.','Geometry hits do not prove bark.','Colors and native animation occupancy do not decide material ownership.','All37 components and2443 pixels remain protected from synthesis.'],cases=rows)
(OUT/'casebook.json').write_text(json.dumps(report,indent=2)+'\n')
html='<meta charset="utf-8"><title>Tree14 complete disputed material domain</title><style>body{background:#292930;color:white;font:16px system-ui;margin:30px}img{max-width:100%;image-rendering:pixelated}section{margin:30px 0}</style><h1>Tree14 complete disputed material domain</h1><p>All 2,443 disputed pixels in 37 connected review units. No automatic material classification. Original source stays visible beside boundaries and isolated pixels.</p>'
for row in rows:html+=f'<section><h2>Case {row["case"]}: {row["pixels"]} pixels</h2><img src="{row["image"]}"></section>'
(OUT/'index.html').write_text(html)
print('Complete casebook:',count,'components,2443pixels')
