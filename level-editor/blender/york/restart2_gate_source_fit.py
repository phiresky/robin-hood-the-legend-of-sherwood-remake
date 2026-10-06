"""Compare diagnostic gate geometry with the exact native sprite alpha."""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement'
base=WORK/'restart2'/sys.argv[1]
source=WORK/'geometry-pass-01/native-state-source-v1'
record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-000')
rows=[]
for state,row,index in [('covered',0,0),('raised',1,44)]:
    frame=record['rows'][row]['frames'][index]
    rgba=Image.open(base/state/'native/native-textured.png').convert('RGBA').resize((220,250),Image.Resampling.BOX)
    pixels=np.asarray(rgba).astype(float)
    candidate=(pixels[:,:,0]>pixels[:,:,1]*1.08)&(pixels[:,:,1]>pixels[:,:,2]*1.15)&(pixels[:,:,3]>127)
    expected=Image.new('L',(220,250));expected.paste(Image.open(source/frame['image']).getchannel('A'),(frame['bbox'][0]-2250,frame['bbox'][1]-780))
    native=np.asarray(expected)>127
    rows.append({'state':state,'candidate_bbox':Image.fromarray(candidate).getbbox(),'source_bbox':expected.getbbox(),
                 'candidate_pixels':int(candidate.sum()),'source_pixels':int(native.sum()),'overlap_pixels':int((candidate&native).sum())})
    overlay=np.zeros((250,220,3),dtype='uint8');overlay[native]=[255,0,255];overlay[candidate]=[0,255,255];overlay[native&candidate]=[255,255,255]
    Image.fromarray(overlay).resize((880,1000),Image.Resampling.NEAREST).save(base/state/'source-silhouette-diagnostic.png')
report={'scope':'Color-segmented diagnostic ochre from 2x native render, box-filtered. This is not an exact source ownership/first-hit guard.',
        'legend':{'magenta':'source only','cyan':'candidate only','white':'overlap'},'states':rows}
(base/'source-silhouette-diagnostic.json').write_text(json.dumps(report,indent=2)+'\n')
sheet=Image.new('RGB',(600,780),'#444444')
raw=Image.open(source/'patch-000/row-0/frame-000.png');flat=Image.new('RGBA',raw.size,'#444444');flat.alpha_composite(raw)
sheet.paste(flat.resize((110,710),Image.Resampling.NEAREST).convert('RGB'),(15,40))
candidate=Image.open(base/'covered/native/native-textured.png').crop((186,176,208,318))
sheet.paste(candidate.resize((110,710),Image.Resampling.NEAREST).convert('RGB'),(160,40))
diagnostic=Image.open(base/'covered/source-silhouette-diagnostic.png').crop((372,352,416,636))
sheet.paste(diagnostic.resize((110,710),Image.Resampling.NEAREST),(310,40))
draw=ImageDraw.Draw(sheet)
for x,label in [(15,'Native alpha'),(160,'Candidate'),(310,'Source/candidate overlap')]:draw.text((x,12),label,fill='white')
sheet.save(base/'gate-lattice-closeup.png')
print(json.dumps(report))
