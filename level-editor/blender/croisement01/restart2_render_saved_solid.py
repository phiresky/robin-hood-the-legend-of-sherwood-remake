"""Render saved candidate geometry through its reviewed camera packet."""
import argparse,json,sys
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha
from render_multiview_asset import render
p=argparse.ArgumentParser();p.add_argument('worker',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.worker.resolve();out=w/'inspection/saved-solid-v1'
assert not out.exists();acquire(slots=2)
try:
 model=w/'model.blend';before=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model))
 manifest=w/'inspection/actual-camera-manifest.json'
 render(manifest,out,modes=('solid',),width=384)
 images=[Image.open(out/f'view-{i}-solid.png').convert('RGB') for i in range(8)];width,height=images[0].size;sheet=Image.new('RGB',(width*4,height*2))
 for i,im in enumerate(images):sheet.paste(im,((i%4)*width,(i//4)*height))
 sheet.save(out/'sheet.png');assert sha(model)==before
 (out/'evidence.json').write_text(json.dumps(dict(model_sha256=before,manifest_sha256=sha(manifest),images={p.name:sha(p) for p in out.glob('*.png')},original_camera_first=True),indent=2)+'\n')
finally:release()
