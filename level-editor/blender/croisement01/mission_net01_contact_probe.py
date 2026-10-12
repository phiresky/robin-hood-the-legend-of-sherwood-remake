"""Measure exact branch intersections of saved net cords without rendering."""
import sys,json,hashlib,math,time,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
sys.path.insert(0,str(Path(__file__).resolve().parent))
from mission_net01_intersections import intersection_points
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
OUT=ROOT/'level-editor/work/active/croisement01/mission-net01-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
acquire(slots=2)
try:
 while int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024<6*1024**3 or shutil.disk_usage(OUT).free<10*1024**3:time.sleep(30)
 pin=json.loads((OUT/'tree04-context.json').read_text());path=ROOT/pin['path'];assert sha(path)==pin['sha256'];bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.window.scene=bpy.data.scenes[pin['scene']];bpy.context.view_layer.update();o=bpy.data.objects['Tree04 decorative wood'];e=o.evaluated_get(bpy.context.evaluated_depsgraph_get());m=e.to_mesh();vs=[e.matrix_world@v.co for v in m.vertices];fs=[tuple(f.vertices)for f in m.polygons];tree=BVHTree.FromPolygons(vs,fs);e.to_mesh_clear()
 worker=OUT/'candidate-v6/model.blend';bpy.ops.wm.open_mainfile(filepath=str(worker));s=math.sin(math.radians(35));c=math.cos(math.radians(35));rows=[]
 for o in bpy.context.scene.objects:
  if o.type!='MESH' or not any(x in o.name for x in ['cord','bag neck','upper left support']):continue
  v=[o.matrix_world@p.co for p in o.data.vertices];f=[tuple(p.vertices)for p in o.data.polygons];pairs=tree.overlap(BVHTree.FromPolygons(v,f));ids=sorted(set(i for _,j in pairs for i in f[j]));coords=[v[i] for i in ids];actual=[Vector(p) for ti,ci in pairs for p in intersection_points(vs,fs[ti],v,f[ci])];rows.append({'object':o.name,'state':o.get('state'),'intersection_pairs':len(pairs),'exact_points':len(actual),'exact_native_y_bounds':[min(-p.y*s-p.z*c for p in actual),max(-p.y*s-p.z*c for p in actual)] if actual else None,'native_bounds':[[min(p.x for p in coords),min(-p.y*s-p.z*c for p in coords)],[max(p.x for p in coords),max(-p.y*s-p.z*c for p in coords)]] if coords else None,'intersecting_vertices':[{'index':i,'world':list(v[i]),'native':[v[i].x,-v[i].y*s-v[i].z*c]} for i in ids]})
 report={'worker_sha256':sha(worker),'tree':pin,'scope':'Saved v6 applied cords share v9 positions; initial cord differs in v9 and is historical context only. Surface intersections, not containment proof.','objects':rows};(OUT/'saved-cord-branch-intersections-v2.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps([{k:v for k,v in r.items() if k!='intersecting_vertices'} for r in rows],indent=2))
finally:release()
