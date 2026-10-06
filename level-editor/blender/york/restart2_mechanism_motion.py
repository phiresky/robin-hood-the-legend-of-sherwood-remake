"""Measure the native winch's travelling round part without assigning runtime coupling."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/york-refinement'
SRC=BASE/'geometry-pass-01/native-state-source-v1'
OUT=BASE/'restart2/gate-mechanism-motion-v1'
if OUT.exists(): raise FileExistsError(OUT)
manifest=json.loads((SRC/'manifest.json').read_text())
record=next(r for r in manifest['records'] if r['id']=='patch-004')
frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')
OUT.mkdir(parents=True)
rows=[]
sheet=Image.new('RGB',(1000,500),'#303840');draw=ImageDraw.Draw(sheet)
for i,f in enumerate(frames):
    path=SRC/f['image'];assert hashlib.sha256(path.read_bytes()).hexdigest()==f['sha256']
    arr=np.array(Image.open(path).convert('RGBA'));x,y,w,h=f['bbox']
    xx=np.arange(w)[None,:]+x;yy=np.arange(h)[:,None]+y
    # Exclude vertical-chain columns and lower crank/frame. Remaining native pixels
    # bound the travelling part's sides, not its depth or material ownership.
    mask=(arr[:,:,3]>127)&(yy<941)&((xx<2395)|((xx>2401)&(xx<2407)))
    ys,xs=np.where(mask)
    bbox=None if len(xs)<4 else [int(xs.min()+x),int(ys.min()+y),int(xs.max()+x+1),int(ys.max()+y+1)]
    rows.append({'frame':i,'source_sha256':f['sha256'],'side_pixels':len(xs),'candidate_round_part_bbox':bbox})
    if i in [0,11,22,33,44]:
        col=[0,11,22,33,44].index(i);tile=Image.new('RGBA',(42,98),'#303840');tile.alpha_composite(Image.fromarray(arr),(x-2389,y-882))
        if bbox:
            d=ImageDraw.Draw(tile);d.rectangle((bbox[0]-2389,bbox[1]-882,bbox[2]-2390,bbox[3]-883),outline='#ff66dd')
        sheet.paste(tile.resize((200,466),Image.Resampling.NEAREST).convert('RGB'),(col*200,28));draw.text((col*200+5,7),f'Frame {i}',fill='white')
sheet.save(OUT/'motion-candidates.png')
report={'status':'Measurement proposal, not physical segmentation or geometry approval','frame_count':45,'frames':rows,'interpretation':['Two near-vertical chain traces and a lower rotating spoke crank require separate reusable components.','A round part travels down the left chain during the transition; a single static replacement mesh cannot reproduce the full motion.','No coupling to gate patch000 established. Native initial sprite is transparent; last transition remains the applied background appearance.','Magenta bounds omit chain columns and may include sparse adjacent pixels. They constrain screen motion only, not physical depth.'],'source_manifest_sha256':hashlib.sha256((SRC/'manifest.json').read_bytes()).hexdigest()}
(OUT/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'output':str(OUT),'sample':rows[::11]}))
