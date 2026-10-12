import sys,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
p=ROOT/'level-editor/work/active/york/restart2/restart59-approved-winch-assembly-v1/model.blend';bpy.ops.wm.open_mainfile(filepath=str(p));bpy.context.view_layer.update();rows=[]
for o in bpy.context.scene.objects:
 if o.type=='MESH':
  v=[o.matrix_world@x.co for x in o.data.vertices];rows.append({'name':o.name,'source_node':o.get('source_node'),'native_patch':o.get('native_patch'),'bounds':[[min(p[i]for p in v),max(p[i]for p in v)]for i in range(3)]})
print(json.dumps(rows,indent=2))
