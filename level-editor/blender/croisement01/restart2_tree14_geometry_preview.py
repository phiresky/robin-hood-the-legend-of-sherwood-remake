"""Inspect saved branch geometry before expensive source projection phases."""
import argparse
import json
import shutil
import sys
from pathlib import Path

import bpy
from PIL import Image

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from evidence_io import sha
from render_multiview_asset import render
p=argparse.ArgumentParser();p.add_argument('--revision',type=int,required=True)
a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
r=ROOT/'level-editor/work/croisement01-refinement/restart2'
w=r/f'tree14-wood-v{a.revision}/assets/croisement01-tree-14'
model=w/'baseline.blend';before=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model))
out=w/'inspection/solid-preview-v1';assert not out.exists()
manifest=r/'tree14-wood-v9-projected/assets/croisement01-tree-14/input/views.json'
packet=json.loads(manifest.read_text())
for view in packet['views']:view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]}
(w/'inspection').mkdir(exist_ok=True);camera_file=w/'inspection/preview-cameras.json';camera_file.write_text(json.dumps(packet,indent=2)+'\n')
render(camera_file,out,modes=('solid',),width=384)
images=[Image.open(out/f'view-{i}-solid.png').convert('RGB') for i in range(8)]
width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
sheet.save(out/'sheet.png');assert sha(model)==before
(out/'evidence.json').write_text(json.dumps(dict(status='PREPROJECTION_GEOMETRY_DIAGNOSTIC_NOT_FINAL_REVIEW',model_sha256=before,camera_manifest_sha256=sha(manifest),original_camera_first=True),indent=2)+'\n')
