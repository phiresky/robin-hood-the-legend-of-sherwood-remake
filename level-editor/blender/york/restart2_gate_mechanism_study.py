"""Separate invariant and changing native winch pixels before constructing parts."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
SRC=WORK/'geometry-pass-01/native-state-source-v1'
DEST=WORK/'restart2/gate-mechanism-source-v1'
if DEST.exists():raise FileExistsError(DEST)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
manifest=json.loads((SRC/'manifest.json').read_text())
record=next(r for r in manifest['records'] if r['id']=='patch-004')
transition=next(r for r in record['rows'] if r['action']=='PatchTransition')
frames=transition['frames'];assert len(frames)==45
left=min(f['bbox'][0] for f in frames);top=min(f['bbox'][1] for f in frames)
right=max(f['bbox'][0]+f['bbox'][2] for f in frames);bottom=max(f['bbox'][1]+f['bbox'][3] for f in frames)
rasters=[]
for frame in frames:
    path=SRC/frame['image'];assert sha(path)==frame['sha256']
    image=Image.new('RGBA',(right-left,bottom-top));image.alpha_composite(Image.open(path).convert('RGBA'),(frame['bbox'][0]-left,frame['bbox'][1]-top));rasters.append(np.asarray(image))
stack=np.stack(rasters);opaque=stack[:,:,:,3]>127
always=opaque.all(axis=0);ever=opaque.any(axis=0)
unchanged=always&np.all(stack==stack[0],axis=(0,3))
changing=ever&~unchanged
DEST.mkdir(parents=True)
Image.fromarray(unchanged.astype('uint8')*255).save(DEST/'invariant-opaque-domain.png')
Image.fromarray(changing.astype('uint8')*255).save(DEST/'changing-domain.png')
scale=5;tw=(right-left)*scale;th=(bottom-top)*scale
sheet=Image.new('RGB',(tw*5,(th+25)*2),'#303840');draw=ImageDraw.Draw(sheet)
for col,index in enumerate((0,11,22,33,44)):
    tile=Image.new('RGBA',(right-left,bottom-top),'#303840');tile.alpha_composite(Image.fromarray(stack[index]));sheet.paste(tile.resize((tw,th),Image.Resampling.NEAREST).convert('RGB'),(col*tw,25));draw.text((col*tw+4,7),f'Native transition {index}',fill='white')
    arr=stack[index].copy();arr[~unchanged]=[0,0,0,0]
    tile=Image.new('RGBA',(right-left,bottom-top),'#303840');tile.alpha_composite(Image.fromarray(arr));sheet.paste(tile.resize((tw,th),Image.Resampling.NEAREST).convert('RGB'),(col*tw,th+50));draw.text((col*tw+4,th+32),'Invariant visible pixels',fill='white')
sheet.save(DEST/'mechanism-source-sheet.png')
report={'status':'Source decomposition only; no mesh/material/runtime approval','native_profile':record['sprite'],
        'source_manifest_sha256':sha(SRC/'manifest.json'),'union_bbox':[left,top,right-left,bottom-top],
        'frame_count':45,'initial':'Transparent1x1 sprite; no initial mechanism geometry inferred from transition alone',
        'invariant_opaque_pixels':int(unchanged.sum()),'always_opaque_pixels':int(always.sum()),'ever_opaque_pixels':int(ever.sum()),'changing_pixels':int(changing.sum()),
        'transition_frame_hashes':[f['sha256'] for f in frames],
        'limitations':['Pixel variation can represent lighting, rotating parts, or occlusion; this is not automatic physical segmentation.','Patch004 independent of gate patch000; synchronized progress and mission reachability not established.','Last transition graphic persists in background; invalid final animation is not disappearance.']}
(DEST/'manifest.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report|{'transition_frame_hashes':'45 bound hashes'}))
