import sys,json,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-mechanism-bounds-v1';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
bpy.ops.wm.open_mainfile(filepath=str(W/'restart59-approved-winch-assembly-v1/model.blend'));scene=bpy.context.scene;rows=[]
for frame in range(45):
 scene.frame_set(frame);bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get()
 for obj in scene.objects:
  if obj.type!='MESH' or obj.get('native_patch')!='patch-004':continue
  o=obj.evaluated_get(deps);v=[o.matrix_world@p.co for p in o.data.vertices];rows.append({'frame':frame,'name':o.name,'bounds':[[min(p[i]for p in v),max(p[i]for p in v)]for i in range(3)]})
OUT.mkdir();(OUT/'bounds.json').write_text(json.dumps(rows));print('Wrote',len(rows),'bounds')
