"""Render solid geometry at the saved original-camera-first eight review views."""
import json,sys
from pathlib import Path
import bpy
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from review_evidence import sha
from render_multiview_asset import render
worker=Path(sys.argv[sys.argv.index('--')+1]).resolve();acquire();before=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));out=worker/'inspection/solid-geometry';assert not out.exists();render(worker/'inspection/actual-camera-manifest.json',out,modes=('solid',),width=256)
images=[Image.open(out/f'view-{i}-solid.png').convert('RGB') for i in range(8)];width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
for i,im in enumerate(images):
 ImageDraw.Draw(im).text((5,5),'0 Native game camera' if i==0 else f'View {i}',fill='white',stroke_width=1,stroke_fill='black');sheet.paste(im,((i%4)*width,(i//4)*height))
sheet.save(out/'sheet.png');assert sha(worker/'model.blend')==before;(out/'evidence.json').write_text(json.dumps(dict(model_sha256=before,views_sha256=sha(worker/'modified/views.json'),sheet_sha256=sha(out/'sheet.png'),native_camera_first=True),indent=2)+'\n')
