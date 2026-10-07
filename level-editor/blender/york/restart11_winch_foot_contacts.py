"""Inspect actual saved support feet against the room floor, without changing geometry."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];VERSION=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'winch-room-physical-v9';BASE=ROOT/'level-editor/work/york-refinement/restart2'/VERSION;OUT=BASE/'foot-contact-audit.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
c=math.cos(math.radians(35));reports=[]
for state in ('transition-00','transition-44'):
 p=BASE/state/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(p));bpy.context.view_layer.update();scene=bpy.context.scene
 floors=[o for o in scene.objects if o.type=='MESH' and (o.get('source_node')=='building-98' or o.name=='Floor contact proxy 98')];assert len(floors)==1
 floor=floors[0];vs=[floor.matrix_world@v.co for v in floor.data.vertices];tree=BVHTree.FromPolygons(vs,[tuple(f.vertices) for f in floor.data.polygons]);rows=[]
 for o in scene.objects:
  if not o.name.startswith(('Angled','Frame foot rail')):continue
  points=[o.matrix_world@v.co for v in o.data.vertices];low=min(v.z for v in points);foot=[v for v in points if v.z<=low+4/c];samples=[]
  for v in foot:
   hit=tree.ray_cast(Vector((v.x,v.y,1000)),Vector((0,0,-1)))
   samples.append({'xy':[v.x,v.y],'z_game':v.z*c,'floor_game':hit[0].z*c if hit[0] is not None else None,'clearance_game':(v.z-hit[0].z)*c if hit[0] is not None else None})
  rows.append({'object':o.name,'minimum_game_z':low*c,'samples':samples,'floor_misses':sum(r['floor_game'] is None for r in samples),'minimum_clearance_game':min(r['clearance_game'] for r in samples if r['clearance_game'] is not None)})
 reports.append({'state':state,'model_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'floor_object':floor.name,'supports':rows})
OUT.write_text(json.dumps({'scope':'Actual saved support vertices within4gameZ of each support minimum, vertical rays against actual floor98 mesh. Negative clearance indicates penetration; no shape approval.','states':reports},indent=2)+'\n')
print(json.dumps([{'state':r['state'],'supports':[{'object':o['object'],'minimum_clearance_game':o['minimum_clearance_game'],'floor_misses':o['floor_misses']} for o in r['supports']]} for r in reports],indent=2))
