"""Rebake ground holes with the same texture-synthesis CLI as volume-fill.ts.

Run normal Python after prepare_ground_reprojection.py. --prepare-only writes
auditable masks; omit it to run the installed Rust CLI and composite the result.
No generated pixel is applied outside the cleanup mask.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from paths import DATA, ROOT

parser=argparse.ArgumentParser()
parser.add_argument('--prepare-only',action='store_true')
args=parser.parse_args()
OUT=ROOT/'level-editor/work/sherwood-refinement/ground-reprojection'
source=Image.open(DATA/'Levels/Day/sherwood.map.png').convert('RGB')
before=Image.open(OUT/'ground-before.png').convert('RGB')
if source.size!=before.size or source.size!=(1920,1088):
    raise RuntimeError('Source map/ground dimensions differ')
regions=json.loads(Path(__file__).with_name('ground_cleanup_regions.json').read_text())
ownership=np.zeros((1088,1920),dtype=bool)
for variant in ['refined','legacy']:
    ownership|=np.array(Image.open(OUT/f'{variant}-ownership.png').getchannel('A'))>16
mask=Image.fromarray(ownership.astype(np.uint8)*255)
draw=ImageDraw.Draw(mask)
for region in regions['erase']:
    draw.polygon([tuple(p) for p in region['points']],fill=255)
mask=mask.filter(ImageFilter.MaxFilter(13))
erase=np.array(mask)>0
known=Image.fromarray((~erase).astype(np.uint8)*255)
donors=Image.new('L',source.size)
draw=ImageDraw.Draw(donors)
for region in regions['donors']:
    draw.polygon([tuple(p) for p in region['points']],fill=255)
sample=Image.fromarray(((np.array(donors)>0)&~erase).astype(np.uint8)*255).filter(ImageFilter.MinFilter(9))
if np.count_nonzero(np.array(sample))<10000:
    raise RuntimeError('Too few audited ground donor pixels')
mask.save(OUT/'cleanup-mask.png')
known.save(OUT/'keep-mask.png')
sample.save(OUT/'donor-mask.png')
source.save(OUT/'source-day.png')
overlay=np.array(source).copy()
overlay[erase]=(overlay[erase]*.40+np.array([235,40,140])*.60).astype(np.uint8)
samples=np.array(sample)>0
overlay[samples]=(overlay[samples]*.35+np.array([40,235,130])*.65).astype(np.uint8)
Image.fromarray(overlay).save(OUT/'mask-audit.png')
report={'geometry_owned_pixels':int(ownership.sum()),'cleanup_pixels':int(erase.sum()),
        'cleanup_fraction':float(erase.mean()),'ground_donor_pixels':int(samples.sum()),
        'method':'texture-synthesis CLI, known-pixel inpaint mask plus audited ground-only donor mask'}
if args.prepare_only:
    print(json.dumps(report))
    raise SystemExit(0)
binary=os.environ.get('TEXTURE_SYNTHESIS',str(Path.home()/'.cargo/bin/texture-synthesis'))
if not Path(binary).is_file():
    raise FileNotFoundError(binary)
command=[binary,'--no-progress','--threads','8','--seed','42','--out',str(OUT/'synthesized-ground.png'),
         '--out-size','1920x1088','--sample-masks',str(OUT/'donor-mask.png'),
         '--inpaint',str(OUT/'keep-mask.png'),'generate',str(OUT/'source-day.png')]
start=time.monotonic()
subprocess.run(command,check=True)
filled=Image.open(OUT/'synthesized-ground.png').convert('RGB')
if filled.size!=source.size:
    raise RuntimeError('Synthesizer output dimensions differ')
# A forest-only pass prevents the large hidden woodland regions from being
# dominated by the brighter grass exemplars used around the clearing.
forest=Image.new('L',source.size)
fd=ImageDraw.Draw(forest)
fd.polygon([(0,0),(1120,0),(1120,420),(940,550),(720,452),(460,455),(280,617),(0,650)],fill=255)
fd.polygon([(1450,0),(1919,0),(1919,650),(1740,650),(1700,410),(1560,236)],fill=255)
forest_erase=(np.array(forest)>0)&erase
rgb=np.array(source).astype(np.float32)
luma=rgb[:,:,0]*.2126+rgb[:,:,1]*.7152+rgb[:,:,2]*.0722
forest_samples=samples&(rgb[:,:,0]>rgb[:,:,1]*1.10)&(rgb[:,:,1]>rgb[:,:,2]*1.08)&(luma<105)
if int(forest_samples.sum())<10000:
    raise RuntimeError('Too few shaded leaf-litter donors for the forest fill')
Image.fromarray((~forest_erase).astype(np.uint8)*255).save(OUT/'forest-keep-mask.png')
Image.fromarray(forest_samples.astype(np.uint8)*255).save(OUT/'forest-donor-mask.png')
forest_command=[binary,'--no-progress','--threads','8','--seed','43','--out',str(OUT/'forest-filled.png'),
                '--out-size','1920x1088','--sample-masks',str(OUT/'forest-donor-mask.png'),
                '--inpaint',str(OUT/'forest-keep-mask.png'),'generate',str(OUT/'synthesized-ground.png')]
subprocess.run(forest_command,check=True)
forest_filled=Image.open(OUT/'forest-filled.png').convert('RGB')
if forest_filled.size!=source.size:
    raise RuntimeError('Forest synthesizer output dimensions differ')
# Rebuild from the original map, not the already-filled legacy texture. The
# latter contains old fill artifacts outside today's geometry silhouettes.
result=np.array(source).copy()
weight=np.array(forest.filter(ImageFilter.GaussianBlur(32))).astype(np.float32)/255
weight*=np.clip((420-np.arange(1088,dtype=np.float32)[:,None])/220,0,1)
mixed=np.rint(np.array(filled)*(1-weight[:,:,None])+np.array(forest_filled)*weight[:,:,None]).astype(np.uint8)
result[erase]=mixed[erase]
Image.fromarray(result).save(OUT/'ground-clean.png')
outside_changes=np.count_nonzero(np.any(result!=np.array(source),axis=2)&~erase)
if outside_changes:
    raise RuntimeError('Composite modified known ground pixels')
report.update(seconds=round(time.monotonic()-start,2),command=command,forest_command=forest_command,
              forest_donor_pixels=int(forest_samples.sum()),outside_mask_changed_pixels=int(outside_changes),
              comparison_reference='Original Day map; legacy filled texture is not reused')
(OUT/'fill-validation.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report))
