"""Replace projection-only diagnostics with protected stored-material cap inputs."""
import sys,json,shutil
from pathlib import Path
import bpy
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render
from prepare_private_texture_inputs import prepare
BASE=OUT/'restart2-state/cleared-fence-inputs-v1';DEST=OUT/'restart2-state/cleared-fence-inputs-v2'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 acquire()
 try:
  DEST.mkdir();shutil.copyfile(BASE/'model.blend',DEST/'model.blend');shutil.copyfile(BASE/'source.png',DEST/'source.png');shutil.copytree(BASE/'modified',DEST/'modified');meta=json.loads((DEST/'modified/views.json').read_text());meta['source_blend']=str(DEST/'model.blend');meta['source_image']=str(DEST/'source.png')
  for v in meta['views']:v['crop']={'left':0,'top':0,'width':384,'height':384}
  write_json(DEST/'modified/views.json',meta);bpy.ops.wm.open_mainfile(filepath=str(DEST/'model.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;scene.render.engine='CYCLES';scene.cycles.samples=24;scene.render.film_transparent=True
  render(DEST/'modified/views.json',DEST/'stored',width=384);sheet=Image.new('RGBA',(1536,768))
  for v in meta['views']:
   i=v['index'];source=DEST/'stored'/f'view-{i}-textured.png';target=DEST/'modified/views'/source.name;shutil.copyfile(source,target);sheet.paste(Image.open(source).convert('RGBA'),((i%4)*384,(i//4)*384))
  sheet.save(DEST/'modified/textured.png');derivation=json.loads((BASE/'derivation.json').read_text());derivation['stored_material_inputs']=True;derivation['prepared_model_sha256']=sha(DEST/'model.blend');derivation['prior_projection_diagnostic']=str(BASE);write_json(DEST/'derivation.json',derivation);prepare(DEST,sha(DEST/'model.blend'),DEST/'private-inputs')
 finally:release()
if __name__=='__main__':main()
