"""Identify the precise inferred surface obstructing a protected bark ray."""
import sys,math,json
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
acquire()
try:
 bpy.ops.wm.open_mainfile(filepath=str(B/'tree10-crown-prototype-v2/worker.blend'))
 ray=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))))
 origin=Vector((833.5,-112.5/math.sin(math.radians(35)),0))+ray*10000
 hits=[]
 for o in bpy.data.scenes['Tree13 isolated wood'].objects:
  if o.type!='MESH':continue
  m=o.data;m.calc_loop_triangles();tree=BVHTree.FromPolygons([o.matrix_world@v.co for v in m.vertices],[list(t.vertices) for t in m.loop_triangles],all_triangles=True)
  p,n,f,d=tree.ray_cast(origin,-ray)
  if p is not None:hits.append(dict(name=o.name,distance=d,face=f,position=list(p),inferred=bool(o.get('inferred_branch'))))
 print(json.dumps(sorted(hits,key=lambda h:h['distance']),indent=2))
finally:release()
