"""Render only missing saved-model states, preserving prior completed evidence."""
import sys,json,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import build_models as build
from render_slots import acquire,release

def main():
 inp=build.q.OUT/'geometry-v5-review-completion-input-v2.json';plan=json.loads(inp.read_text());build.DEST=build.q.ROOT/plan['output'];assert not build.DEST.exists();build.guard();build.DEST.mkdir()
 for row in plan['missing']:
  model=build.q.ROOT/row['model']['path'];assert build.q.pin(model)==row['model'];build.guard();build.bpy.ops.wm.open_mainfile(filepath=str(model));objects=[o for o in build.bpy.context.scene.objects if o.type=='MESH'];obj=next(o for o in objects if o['source_phase']==row['phase'])
  for o in objects:o.hide_render=o!=obj;o.hide_viewport=o!=obj
  folder=build.DEST/model.parent.name/row['phase'];folder.mkdir(parents=True);build.render(obj,folder);assert build.q.pin(model)==row['model']
  (folder/'completed.json').write_text(json.dumps(dict(model=row['model'],phase=row['phase'],images=[build.q.pin(p)for p in sorted(folder.glob('*.png'))]),indent=2)+'\n')
 (build.DEST/'report.json').write_text(json.dumps(dict(status='SAVED_MODEL_TRANSITION_VIEWS_COMPLETE_REVIEW_PENDING',input=build.q.pin(inp),source_report=plan['source_report'],states=[json.loads(p.read_text())for p in sorted(build.DEST.glob('*/*/completed.json'))]),indent=2)+'\n')
if __name__=='__main__':
 def available():return int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024
 while True:
  while available()<14*1024**3:print('Waiting14GiB before shared queue',available(),flush=True);time.sleep(30)
  acquire(slots=2)
  try:
   if available()<10*1024**3:continue
   main();break
  finally:release()
