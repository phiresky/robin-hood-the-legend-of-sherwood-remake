"""Read-only native root ray intersections and vertical support diagnostics."""
import json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS
from render_slots import acquire
from evidence_io import sha
r=OUT/'restart2/tree01-v2';w=r/'assets/croisement01-tree-01';acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());verts=[];faces=[];owners=[]
for o in bpy.data.collections[cfg['collection_name']].all_objects:
 if o.type!='MESH' or o.get('source_node') not in {'ground'}|{f'building-{i:03}' for i in [*range(10),*range(76,81)]}:continue
 offset=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in p.vertices) for p in o.data.polygons);owners.extend([o.get('source_node')]*len(o.data.polygons))
tree=BVHTree.FromPolygons(verts,faces);ray=Vector((0,-COS,SIN));results=[]
for x,y in [(55,300),(55,310),(55,318),(55,322),(45,318),(65,318)]:
 origin=Vector((x,-y/SIN,0))+ray*5000;hits=[]
 for i in range(20):
  hit,normal,index,distance=tree.ray_cast(origin,-ray,20000)
  if hit is None:break
  hits.append(dict(point=list(hit),normal=list(normal),owner=owners[index]));origin=hit-ray*.01
 results.append(dict(native=[x,y],hits=hits))
(r/'support-ray-probe.json').write_text(json.dumps(dict(model_sha256=sha(w/'model.blend'),rays=results),indent=2)+'\n');print(json.dumps(results))
