"""Wider eight-view material supplement retaining the frozen native first direction."""
import argparse,json,sys
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from evidence_io import sha,write_json
from render_multiview_asset import render
from render_slots import acquire,release
from tree_geometry import RAY

def main():
 p=argparse.ArgumentParser();p.add_argument('--workspace',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.workspace.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False);digest=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update();packet=json.loads((w/'modified/views.json').read_text());direction=Matrix(packet['views'][0]['camera_matrix_world']).to_3x3()@Vector((0,0,1))
 if direction.normalized().dot(RAY)<.999999:raise ValueError('First view is not native direction')
 for v in packet['views']:v['ortho_scale']*=1.25;v['crop']=dict(width=384,height=384)
 scene=bpy.data.scenes[packet['scene_name']];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.world=bpy.data.worlds.new('Neutral wider actual');scene.world.color=(.1,.1,.1);write_json(out/'views.json',packet);render(out/'views.json',out/'views',modes=('solid','textured'),width=384)
 for mode in ['solid','textured']:
  sheet=Image.new('RGB',(1536,768))
  for i in range(8):sheet.paste(Image.open(out/f'views/view-{i}-{mode}.png'),((i%4)*384,(i//4)*384))
  sheet.save(out/f'{mode}.png')
 write_json(out/'evidence.json',dict(model_sha256=digest,worker=str(w),native_first=True,orthographic_scale_factor=1.25,transparent_bounces=256,files={n:sha(out/n) for n in ['solid.png','textured.png','views.json']}))
 if sha(w/'model.blend')!=digest:raise ValueError('Model changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
