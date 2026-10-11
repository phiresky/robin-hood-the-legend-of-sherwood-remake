"""Measure private soil support; the native obstacle foot is not observed ground."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
base=ROOT/'level-editor/work/croisement01-refinement/restart2'
assert shutil.disk_usage(base).free>10*1024**3
available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
assert available>6*1024**3
source=base/'tree05-regional-banks-v18/assets/croisement01-tree-05/model.blend'
out=base/'tree07-soil-support-v1';out.mkdir(exist_ok=False)
bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update()
verts=[];faces=[];owners=[]
for obj in bpy.context.scene.objects:
 if obj.type!='MESH' or obj.get('source_node') not in ['ground',*[f'building-{i:03d}' for i in range(10)]]:continue
 offset=len(verts);verts.extend(obj.matrix_world@v.co for v in obj.data.vertices)
 faces.extend(tuple(offset+i for i in p.vertices) for p in obj.data.polygons);owners.extend([obj.get('source_node')]*len(obj.data.polygons))
bvh=BVHTree.FromPolygons(verts,faces);samples=[]
for x in [447,458,469]:
 for y in [-318,-307,-296]:
  origin=Vector((x,y,1000));hits=[]
  for _ in range(50):
   hit,normal,index,distance=bvh.ray_cast(origin,Vector((0,0,-1)),2000)
   if hit is None:break
   hits.append(dict(point=list(hit),normal=list(normal),node=owners[index]));origin=hit-Vector((0,0,.01))
  samples.append(dict(x=x,y=y,hits=hits))
report=dict(status='PRIVATE_SUPPORT_MEASUREMENT_NOT_APPROVED_TERRAIN',model_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),samples=samples,limitations='Regional bank v18 is an inferred private hypothesis. Upward support provides a continuous placement candidate, not native soil evidence or a new gameplay association.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
