"""Resume the saved provisional crown diagnostic without rebuilding geometry."""
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
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
out=ROOT/'level-editor/work/croisement01-refinement/restart2/tree14-crown94-geometry-v1'
checkpoint=json.loads((out/'checkpoint.json').read_text())
assert sha(out/'model.blend')==checkpoint['model_sha256']
bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'))
packet=json.loads((out/'views.json').read_text());packet['render_object_names']=None
manifest=out/'diagnostic-cameras-v2.json';assert not manifest.exists()
manifest.write_text(json.dumps(packet,indent=2)+'\n')
render(manifest,out/'material-views-v2',modes=('textured',),width=256)
images=[Image.open(out/f'material-views-v2/view-{i}-textured.png').convert('RGB') for i in range(8)]
width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
for i,image in enumerate(images):sheet.paste(image,((i%4)*width,(i//4)*height))
sheet.save(out/'material-eight-v2.png')
assert sha(out/'model.blend')==checkpoint['model_sha256']
