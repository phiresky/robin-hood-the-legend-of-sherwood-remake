"""Limit contour correction colors to the explicitly observed missing edge centers."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from approved_texture_stage import geometry

def main():
 number=int(sys.argv[sys.argv.index('--')+1]);source=ROOT/f'tree{number}-contour-v1/model.blend';out=ROOT/f'tree{number}-contour-v2';out.mkdir(exist_ok=False);digest=sha(source)
 bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;names={o.name for o in scene.objects if o.type=='MESH'};before={o.name:geometry(o)for o in scene.objects if o.name in names}
 item=next(x for x in json.loads((OUT/'review-mask-inventory.json').read_text())['masks']if x['index']==number);ox,oy=item['box_top_left'];w,h=item['box_size'];mask=np.array(Image.open(ROOT/f'source-audit-v1/exposed-{number}.png').convert('L'))[oy:oy+h,ox:ox+w]
 image=next(im for im in bpy.data.images if im.name.startswith(f'native{number}'));rgba=np.array(Image.open(ROOT/f'tree{number}-contour-v1/native{number}.png').convert('RGBA'));rgba[:,:,3]=mask;path=out/f'native{number}.png';Image.fromarray(rgba).save(path)
 replacement=bpy.data.images.load(str(path));replacement.pack()
 for mat in bpy.data.materials:
  if mat.use_nodes:
   for node in mat.node_tree.nodes:
    if node.type=='TEX_IMAGE'and node.image==image:node.image=replacement
 assert before=={o.name:geometry(o)for o in scene.objects if o.name in names};bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
 write_json(out/'overlay-scope.json',dict(source_sha256=digest,model_sha256=sha(out/'model.blend'),observed_pixels=int((mask>0).sum()),scope='Native color override restricted to explicitly missing source edge centers; unchanged geometry and existing appearance fallback.',source_image=str(path)))
 assert sha(source)==digest
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
