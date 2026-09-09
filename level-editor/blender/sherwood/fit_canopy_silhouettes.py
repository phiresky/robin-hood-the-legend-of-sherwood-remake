"""Measure source occupancy and record sparse extra spray samples in missed areas.

Run normal Python after render_canopy_masks.py. The --measure-only option does
not alter the tracked fit samples. These are reference-derived 2D coordinates,
not recovered depth measurements.
"""
import json
import sys
from pathlib import Path
import numpy as np
from PIL import Image
from paths import ROOT

out=ROOT/'level-editor/work/sherwood-refinement/branch-canopies'
refs=out.parent/'animation-references'
records=json.loads((refs/'manifest.json').read_text())['assets']
fit_path=Path(__file__).with_name('foliage_fit_samples.json')
fit=json.loads(fit_path.read_text()) if fit_path.exists() else {}
report={}
for r in records:
    if r['kind']!='tree':continue
    sprite=np.array(Image.open(refs/r['first_png']).convert('RGBA'))[:,:,3]>127
    expected=np.zeros((1088,1920),bool)
    x,y=r['left'],r['top'];h,w=sprite.shape
    X0,Y0=max(0,x),max(0,y);X1,Y1=min(1920,x+w),min(1088,y+h)
    expected[Y0:Y1,X0:X1]=sprite[Y0-y:Y1-y,X0-x:X1-x]
    image=np.array(Image.open(out/(r['profile'].split(' - ')[1].lower()+'-alpha.png')).convert('RGBA'))
    actual=image[:,:,3]>127;missing=expected&~actual
    added=[]
    for yy in range(Y0,Y1,4):
        for xx in range(X0,X1,4):
            candidates=np.argwhere(missing[yy:min(yy+4,Y1),xx:min(xx+4,X1)])
            if len(candidates):
                dy,dx=candidates[len(candidates)//2];added.append([int(xx+dx),int(yy+dy)])
    report[r['profile']]={'source_pixels':int(expected.sum()),
        'covered_fraction':float((expected&actual).sum()/expected.sum()),
        'extra_pixels':int((actual&~expected).sum()),'proposed_fill_sprays':len(added)}
    if '--measure-only' not in sys.argv:
        existing={tuple(p) for p in fit.get(r['profile'],[])}
        existing.update(tuple(p) for p in added);fit[r['profile']]=[list(p) for p in sorted(existing)]
if '--measure-only' not in sys.argv:fit_path.write_text(json.dumps(fit,separators=(',',':'))+'\n')
(out/'silhouette-validation.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
