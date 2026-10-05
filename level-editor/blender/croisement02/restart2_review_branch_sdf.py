"""Freeze existing close cameras for private branch union shape review."""
import argparse,json,sys
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import write_json,sha
from render_slots import acquire,release
from render_multiview_asset import render

def main():
 p=argparse.ArgumentParser();p.add_argument('index',type=int);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);index=a.index;root=OUT/f'restart2-wood/tree{index}-branch-sdf-v1';out=root/'solid-review';out.mkdir(exist_ok=False);digest=sha(root/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(root/'model.blend'));bpy.context.view_layer.update();objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==f'croisement02-tree-{index}' and o.get('projection_component')!='crown'];transforms={o:o.matrix_world.copy() for o in objects};scene=bpy.data.scenes.new('Private continuous branch review');scene.world=bpy.data.worlds.new('Neutral branch environment');scene.world.color=(.12,.12,.12);bpy.context.window.scene=scene;names=[]
 for original in objects:
  obj=original.copy();obj.parent=None;obj.matrix_world=transforms[original];obj.hide_render=False;scene.collection.objects.link(obj);names.append(obj.name)
 packet=json.loads((OUT/f'restart2-wood/tree{index}-boundary-review-v3/after-views.json').read_text());packet['scene_name']=scene.name;packet['object_names']=names;packet.pop('render_object_names',None);write_json(out/'views.json',packet);render(out/'views.json',out/'views',modes=('solid',),width=256);sheet=Image.new('RGB',(1024,512))
 for i in range(8):sheet.paste(Image.open(out/f'views/view-{i}-solid.png'),((i%4)*256,(i//4)*256))
 sheet.save(out/'solid.png');write_json(out/'evidence.json',dict(model_sha256=digest,sheet_sha256=sha(out/'solid.png'),status='Private shape only, not current source appearance'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
