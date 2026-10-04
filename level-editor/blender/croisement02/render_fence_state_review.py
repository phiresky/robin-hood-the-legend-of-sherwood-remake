"""Render a complete separate state packet, without changing the covered approval."""
import sys,json,hashlib
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from render_multiview_asset import render
from catalog import OUT

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=Path(sys.argv[sys.argv.index('--')+1]).resolve();dest=root/'geometry-review';dest.mkdir(exist_ok=False)
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(root/'worker.blend'))
  exp=OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment';frames=json.loads((exp/'views.json').read_text());scene=bpy.data.scenes[frames['scene_name']];bpy.context.window.scene=scene
  originals=[scene.objects[n] for n in frames['object_names']];copies=[o for o in scene.objects if o.get('state_recipe')=='croisement02-chariot02-barriere-source-domain'];assert len(copies)==2
  for o in originals:o.hide_render=True
  for o in copies:o.hide_render=False
  frames['object_names']=[o.name for o in copies];frames.pop('texture_receiver_object_names',None);frames['tile_size']=[384,384]
  for v in frames['views']:v['crop']={'left':0,'top':0,'width':384,'height':384}
  (dest/'views.json').write_text(json.dumps(frames,indent=2)+'\n');scene.render.engine='CYCLES';scene.cycles.samples=24
  render(dest/'views.json',dest/'views',modes=('textured','solid'),width=384)
  for mode in ('solid','textured'):
   sheet=Image.new('RGB',(1536,768),'#222')
   for i in range(8):
    image=Image.open(dest/'views'/f'view-{i}-{mode}.png').convert('RGB');assert image.size==(384,384);sheet.paste(image,((i%4)*384,(i//4)*384))
   sheet.save(dest/f'{mode}.png')
  records=json.loads((OUT/'user-feedback.json').read_text())['records'];approval=[r for r in records if r['asset_id']=='croisement02-south-field-wattle-fence'][-1];assert approval['decision']=='approved'
  receipt=dict(model_sha256=sha(root/'worker.blend'),covered_geometry_approval=approval,covered_texture_approval='separate; pending at candidate creation',state_id='croisement02-south-field-wattle-fence-cleared-state',scope='New scoped cleared geometry only; covered geometry retains its independent approval. Terminal ground appearance is excluded and remains integration work.',artifacts={str(p.relative_to(root)):sha(p) for p in sorted(root.rglob('*')) if p.is_file() and p.name!='evidence.json'})
  (dest/'evidence.json').write_text(json.dumps(receipt,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
