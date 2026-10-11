"""Check that inferred room boundaries enclose interior points except the entrance."""
import json,math,sys,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-enclosure-v19';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
model=W/'restart61-room-interior-v19/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();vs=[];fs=[]
for o in bpy.context.scene.objects:
 if not o.get('room_receiver'):continue
 o.data.calc_loop_triangles();offset=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices);fs.extend(tuple(offset+i for i in t.vertices)for t in o.data.loop_triangles)
bvh=BVHTree.FromPolygons(vs,fs,all_triangles=True);source=Image.open(W/'restart61-room-interior-v18/revealed-source-domain.png');rows=[];s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
for center in [(2405,-1850,160),(2394,-1860,145),(2410,-1835,220)]:
 p=Vector(center);escapes=[];good=0;hits=0
 for i in range(4096):
  z=1-2*(i+.5)/4096;angle=i*math.pi*(3-math.sqrt(5));d=Vector((math.sqrt(1-z*z)*math.cos(angle),math.sqrt(1-z*z)*math.sin(angle),z));hit=bvh.ray_cast(p,d,1000)[0]
  if hit is not None:hits+=1;continue
  if d.y<0:
   q=p+d*((-1894-p.y)/d.y);x=int(math.floor(q.x))-2353;y=int(math.floor(-q.y*s-q.z*c))-783
   if 0<=x<source.width and 0<=y<source.height and source.getpixel((x,y))[3]>0:good+=1;continue
  escapes.append(list(d))
 rows.append({'origin':center,'rays':4096,'enclosed_rays':hits,'source_entrance_escape_rays':good,'unexpected_escape_count':len(escapes),'unexpected_examples':escapes[:10]})
OUT.mkdir();report={'status':'PASS_SAMPLED_ENCLOSURE'if not any(r['unexpected_escape_count']for r in rows)else'HOLD_ROOM_LEAKS','model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'rows':rows,'limits':'12,288 uniformly distributed directions from three room interior points; not a topological proof. Escape is permitted only through the projected source entrance on the front plane.'};(OUT/'report.json').write_text(json.dumps(report,indent=2));print(json.dumps(report))
