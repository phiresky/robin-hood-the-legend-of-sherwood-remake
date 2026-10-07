"""Verify the moved support root remains inside its unchanged stem volume."""
import sys,json
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2/tree06-crown-prototype-v2'
def inside(tree,p,d):
 count=0
 for _ in range(100):
  hit=tree.ray_cast(p,d)[0]
  if hit is None:return bool(count%2)
  count+=1;p=hit+d*.001
 raise AssertionError('Unbounded support contact ray')
def main():
 assert not (B/'support-contact.json').exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(B/'worker.blend'));scene=bpy.data.scenes['Tree13 isolated wood'];support=scene.objects['Inferred cluster support 4'];center=sum((support.matrix_world@v.co for v in list(support.data.vertices)[:8]),Vector())/8;results={}
  for o in scene.objects:
   if o.type!='MESH' or not o.name.startswith('Tree06 private stem'):continue
   tree=BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons]);results[o.name]=[inside(tree,center,d.normalized()) for d in (Vector((1,.321,.123)),Vector((-.257,1,.173)),Vector((.139,-.231,1)))]
  assert any(all(v) for v in results.values()),results
  write_json(B/'support-contact.json',dict(status='PASS inferred support root remains inside unchanged stem',model_sha256=sha(B/'worker.blend'),support=support.name,root_center=list(center),parity_by_stem=results,limits=['Finite support-root attachment proof, not full terrain contact.']))
 finally:release()
if __name__=='__main__':main()
