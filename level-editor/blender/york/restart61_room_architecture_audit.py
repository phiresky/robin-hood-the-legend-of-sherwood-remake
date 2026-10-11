"""Extract unchanged room and neighboring castle geometry for architectural diagnosis."""
import sys,json,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-architecture-audit-v1';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
OUT.mkdir();rows=[]
for path,kind in [(W/'restart61-room-interior-v13/model.blend','candidate'),(W.parent/'baseline/york-baseline.blend','neighbor-baseline')]:
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
 for o in bpy.context.scene.objects:
  if o.type!='MESH':continue
  vs=[list(o.matrix_world@v.co)for v in o.data.vertices]
  if not vs:continue
  bounds=[[min(p[i]for p in vs),max(p[i]for p in vs)]for i in range(3)]
  if kind=='neighbor-baseline' and not(bounds[0][0]<2550 and bounds[0][1]>2250 and bounds[1][0]<-1650 and bounds[1][1]>-2000 and bounds[2][1]>100):continue
  o.data.calc_loop_triangles();rows.append({'kind':kind,'name':o.name,'source':o.get('source_node'),'receiver':bool(o.get('room_receiver')),'cover':o.get('state_recipe')=='initial_only','bounds':bounds,'vertices':vs,'triangles':[list(t.vertices)for t in o.data.loop_triangles]})
report={'model_sha256':hashlib.sha256((W/'restart61-room-interior-v13/model.blend').read_bytes()).hexdigest(),'baseline_sha256':hashlib.sha256((W.parent/'baseline/york-baseline.blend').read_bytes()).hexdigest(),'objects':rows};(OUT/'geometry.json').write_text(json.dumps(report));print([(r['name'],r['source'],r['bounds'])for r in rows if r['kind']=='neighbor-baseline'])
