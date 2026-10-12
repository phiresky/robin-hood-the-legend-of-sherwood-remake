"""Measure private source-ray terrain support for the twisted northern tree."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
base=ROOT/'level-editor/work/active/croisement01/restart2';source=base/'tree05-regional-banks-v18/assets/croisement01-tree-05/model.blend';out=base/'tree14-source-ground-support-v1';out.mkdir(exist_ok=False)
bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();v=[];f=[];owners=[]
for o in bpy.context.scene.objects:
 if o.type!='MESH' or o.get('source_node') not in ['ground',*[f'building-{i:03d}' for i in range(10)]]:continue
 off=len(v);v.extend(o.matrix_world@p.co for p in o.data.vertices);f.extend(tuple(off+i for i in p.vertices) for p in o.data.polygons);owners.extend([o.get('source_node')]*len(o.data.polygons))
tree=BVHTree.FromPolygons(v,f);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));samples=[]
for x,y in [(850,405),(850,412),(851,395),(794,409),(797,435),(914,402),(892,391),(829,397)]:
 origin=Vector((x,-y/s,0))+ray*5000;hits=[]
 for _ in range(40):
  hit,normal,index,distance=tree.ray_cast(origin,-ray,10000)
  if hit is None:break
  hits.append(dict(point=list(hit),normal=list(normal),node=owners[index],upward=normal.z>.35));origin=hit-ray*.02
 samples.append(dict(source=[x,y],hits=hits))
report=dict(status='PRIVATE_TERRAIN_SUPPORT_DIAGNOSTIC_NOT_APPROVAL',model_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),samples=samples,limitations='Terrain009 remains an archived/inferred surface. Source-ray support is a placement hypothesis, not evidence that all root depths share one plane. Root centerlines must remain full-volume and terrain transitions need separate review.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
