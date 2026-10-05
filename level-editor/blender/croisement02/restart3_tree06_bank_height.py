"""Read-only vertical bank support audit for an inferred buried root collar."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json

def main():
 p=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
 bpy.ops.wm.open_mainfile(filepath=str(p));bpy.context.view_layer.update();trees=[]
 for o in bpy.context.scene.objects:
  if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank':
   o.data.calc_loop_triangles();trees.append(BVHTree.FromPolygons([tuple(o.matrix_world@v.co)for v in o.data.vertices],[tuple(t.vertices)for t in o.data.loop_triangles],all_triangles=True))
 rows=[]
 for y in range(-1040,-939,5):
  for x in range(600,706,5):
   hits=[t.ray_cast(Vector((x,y,2000)),Vector((0,0,-1)),4000)[0]for t in trees];z=[h.z for h in hits if h is not None];rows.append(dict(x=x,y=y,z=max(z)if z else None))
 write_json(OUT/'restart3-tree06-root/bank-heights.json',dict(bank_sha256=sha(p),samples=rows));print([(r['y'],min([q['z']for q in rows if q['y']==r['y']and q['z']is not None],default=None),max([q['z']for q in rows if q['y']==r['y']and q['z']is not None],default=None))for r in rows if r['x']==600])
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
