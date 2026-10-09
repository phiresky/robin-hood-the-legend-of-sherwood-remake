"""Render fixed-camera provenance after the exact bark repair, without model edits."""
import json,shutil,sys,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire,release
from render_texture_coverage import inspect
R=ROOT/'level-editor/work/croisement01-refinement/restart2';O=R/'approved-tree02-fill-v1/croisement01-tree-02/baked-v4-bounded-bark-gaps'
assert shutil.disk_usage(R).free>=10*1024**3+4*1024**2;assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();before=sha(O/'worker.blend');acquire()
@bpy.app.handlers.persistent
def threads(scene,*_):scene.render.threads_mode='FIXED';scene.render.threads=2
bpy.app.handlers.render_pre.append(threads)
try:
 validation=json.loads((O/'validation.json').read_text());(O/'layer-0.json').write_text(json.dumps(validation['layers'][0],indent=2)+'\n');manifest=json.loads((O/'actual-review-v1/inspection/actual-camera-manifest.json').read_text());manifest['tile_size']=[384,384]
 for view in manifest['views']:view['crop']={'width':384,'height':384}
 packet=O/'coverage-views-384.json';packet.write_text(json.dumps(manifest,indent=2)+'\n');result=inspect(packet,O,O/'coverage-v1-384');assert sha(O/'worker.blend')==before;assert sum(p.stat().st_size for p in O.rglob('*') if p.is_file() and not p.is_symlink())<=64*1024**2;print(json.dumps(result['views']),flush=True)
finally:release()
