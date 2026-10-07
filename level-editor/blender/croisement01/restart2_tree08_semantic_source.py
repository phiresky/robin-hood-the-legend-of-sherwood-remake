"""Private material-class proposal for overlapping native bark/canopy artwork."""
import argparse,hashlib,json
from pathlib import Path
import numpy as np
from scipy.ndimage import label,convolve
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];base=ROOT/'level-editor/work/croisement01-refinement/baseline';p=argparse.ArgumentParser();p.add_argument('output',type=Path);a=p.parse_args();a.output.mkdir(exist_ok=False);rows={r['index']:r for r in json.loads((base/'masks/manifest.json').read_text())['masks']};src=Image.open(base/'covered.png').convert('RGB');masks={}
for index in [8,93]:
 canvas=Image.new('L',src.size);canvas.paste(Image.open(base/'masks'/rows[index]['png']),tuple(rows[index]['box_top_left']));masks[index]=np.asarray(canvas)>0
shared=masks[8]&masks[93];rgb=np.asarray(src);values=rgb.astype(float);R,G,B=values.transpose(2,0,1);v=values.max(2);sat=(v-values.min(2))/np.maximum(v,1)
# These are source-specific candidate seeds, never an automatic material rule.
# Darkness alone is insufficient: isolated dark flecks surrounded by bright
# leaves remain ambiguous, while connected gray/brown branch cores survive.
leaf=shared&(v>=100)&(sat>=.5)&(G>=R*.53)&(G>=B*1.45)
bark=shared&((v<=70)|(sat<=.4))&~leaf
for target,minimum in [(leaf,3),(bark,4)]:
 components,count=label(target,np.ones((3,3)));sizes=np.bincount(components.ravel());target&=sizes[components]>=minimum
leaf_neighbors=convolve(leaf.astype('uint8'),np.ones((3,3),dtype='uint8'),mode='constant');bark&=leaf_neighbors<5
ambiguous=shared&~leaf&~bark;assert np.array_equal(bark.astype('uint8')+leaf.astype('uint8')+ambiguous.astype('uint8'),shared.astype('uint8'))
x,y=rows[8]['box_top_left'];width,height=rows[8]['box_size'];bounds=(x,y,x+width,y+height);records={}
for name,domain,color in [('bark-core-proposal',bark,(60,190,255)),('gold-leaf-cluster-proposal',leaf,(255,160,40)),('ambiguous-edge-and-shadow',ambiguous,(240,70,210))]:
 path=a.output/(name+'.png');Image.fromarray((domain*255).astype('uint8')).crop(bounds).save(path);visible=src.convert('RGBA');visible.putalpha(Image.fromarray((domain*255).astype('uint8')));visible=visible.crop(bounds);visible.save(a.output/(name+'-rgba.png'));bg=Image.new('RGBA',visible.size,(45,45,45,255));bg.alpha_composite(visible);bg.convert('RGB').save(a.output/(name+'-source.png'));records[name]=dict(pixels=int(domain.sum()),mask_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
colors=np.asarray(src).copy();colors[bark]=(colors[bark]*.35+np.array([60,190,255])*.65).astype('uint8');colors[leaf]=(colors[leaf]*.35+np.array([255,160,40])*.65).astype('uint8');colors[ambiguous]=(colors[ambiguous]*.35+np.array([240,70,210])*.65).astype('uint8')
for name,box,scale in [('full',(330,0,810,325),1),('central-forks',(470,125,610,245),3),('left-bough',(350,80,495,205),3),('upper-forks',(455,0,635,135),3),('right-crossing',(645,120,775,285),3)]:
 original=src.crop(box);marked=Image.fromarray(colors).crop(box);cw,ch=original.width*scale,original.height*scale;sheet=Image.new('RGB',(cw*2,ch+48),'#222222');sheet.paste(original.resize((cw,ch),Image.Resampling.NEAREST),(0,48));sheet.paste(marked.resize((cw,ch),Image.Resampling.NEAREST),(cw,48));draw=ImageDraw.Draw(sheet);draw.text((5,5),'Untouched source',fill='white');draw.text((cw+5,5),'Source-class proposal; ownership still separate',fill='white');draw.text((5,25),'Blue bark cores / orange gold-leaf clusters / magenta ambiguous',fill='white');sheet.save(a.output/(name+'.png'))
record=dict(status='Candidate material classification requiring visual review; no geometry or user approval',shared_native_pixels=int(shared.sum()),native_masks=[8,93],native_bounds=bounds,source_sha256=hashlib.sha256((base/'covered.png').read_bytes()).hexdigest(),classes=records,partition_exact=True,source_rgba_unchanged=True,method='Local source-specific chroma/value seeds, minimum connected support, isolated dark flecks next to leaf clusters deferred. Must be checked against continuous branch shapes in untouched crops; not a general color-to-material rule.',limitations=['Dark leaf shadows can resemble bark; manual semantic review required','Tiny gold bark highlights can resemble leaves; inspect source clusters before approval','Material classification does not resolve Tree08/14 or crown93/94 ownership','Ambiguous pixels are preserved for later local decisions, not deleted']);(a.output/'classification.json').write_text(json.dumps(record,indent=2)+'\n');print({name:row['pixels'] for name,row in records.items()})
