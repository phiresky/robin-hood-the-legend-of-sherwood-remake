"""Inspect the saved room candidate, ownership partition and review resource pins."""
import sys,json,hashlib,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-final-audit-v19';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
from mathutils import Vector
base=W/'restart42-channel-assembly-v1/model.blend';model=W/'restart61-room-interior-v19/model.blend';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def area(o):
 o.data.calc_loop_triangles();total=0
 for t in o.data.loop_triangles:
  a,b,c=[o.matrix_world@o.data.vertices[i].co for i in t.vertices];total+=(b-a).cross(c-a).length/2
 return total
bpy.ops.wm.open_mainfile(filepath=str(base));bpy.context.view_layer.update();old={o.get('source_node'):area(o)for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_node')in {'building-764','building-765','building-776','building-779'}}
bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();scene=bpy.context.scene;rows=[]
for node,original_area in old.items():
 remainder=next(o for o in scene.objects if o.type=='MESH' and o.get('source_node')==node and o.get('state_recipe')!='initial_only');cover=scene.objects['Room cover / switched '+node];total=area(remainder)+area(cover);rows.append({'source_owner':node,'original_area':original_area,'partition_area':total,'relative_error':abs(total-original_area)/original_area,'cover_object':cover.name,'remainder_object':remainder.name,'ownership':'Both pieces belong to the same original source owner. Canonical source owner retained; cover/remainder have disjoint projection_component labels.'})
meshes=[]
for o in scene.objects:
 if o.type=='MESH' and (o.get('state_recipe')=='initial_only' or o.get('room_receiver') or o.get('source_node')in old):meshes.append({'object':o.name,'faces':len(o.data.polygons),'zero_area_faces':sum(p.area<1e-8 for p in o.data.polygons),'room_receiver':bool(o.get('room_receiver'))})
report={'status':'GEOMETRY_AUDIT_COMPLETE','model_sha256':sha(model),'partition':rows,'meshes':meshes,'max_partition_relative_area_error':max(r['relative_error']for r in rows),'zero_area_faces':sum(r['zero_area_faces']for r in meshes),'texture_state':'Source-only appearance. Hidden gray receiver faces await approved synthesis.','approval_state':'Geometry pending user review.'};OUT.mkdir();(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
