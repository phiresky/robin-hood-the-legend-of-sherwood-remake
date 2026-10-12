"""Check inferred room bounds against every approved moving-winch pose."""
import json,hashlib,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-winch-contact-v13';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
model=W/'restart61-room-interior-v13/model.blend';winch=W/'restart59-approved-winch-assembly-v1/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();boxes={}
for o in bpy.context.scene.objects:
 if o.get('room_receiver'):
  vv=[o.matrix_world@v.co for v in o.data.vertices];boxes[o.name]=[[min(v[i]for v in vv),max(v[i]for v in vv)]for i in range(3)]
bpy.ops.wm.open_mainfile(filepath=str(winch));scene=bpy.context.scene;violations=[];feet=[];rows=[]
for frame in range(45):
 scene.frame_set(frame);bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();count=0
 for obj in scene.objects:
  if obj.type!='MESH' or obj.get('native_patch')!='patch-004':continue
  o=obj.evaluated_get(deps);vv=[o.matrix_world@v.co for v in o.data.vertices];bounds=[[min(v[i]for v in vv),max(v[i]for v in vv)]for i in range(3)];count+=1
  for name,box in boxes.items():
   overlap=[min(bounds[i][1],box[i][1])-max(bounds[i][0],box[i][0])for i in range(3)]
   if min(overlap)>.005:violations.append({'frame':frame,'object':obj.name,'receiver':name,'aabb_overlap':overlap})
  if 'brace' in obj.name.lower()or'foot rail'in obj.name.lower():feet.append(abs(bounds[2][0]-boxes['Room inferred Floor'][2][1]))
 rows.append({'frame':frame,'components':count})
OUT.mkdir();report={'status':'PASS_CONSERVATIVE_AABB_CLEARANCE'if not violations else'HOLD_CONTACT_OVERLAPS','model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'winch_sha256':hashlib.sha256(winch.read_bytes()).hexdigest(),'poses':rows,'inferred_receiver_bounds':boxes,'positive_overlap_tolerance':.005,'violations':violations,'support_floor_max_gap':max(feet),'scope':'All 45 approved winch poses versus seven new room volumes. Does not certify intersections with existing static gatehouse or gate animation.'};(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
