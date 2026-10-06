"""Read-only exact rig-to-loop overlap and point parity at the upper attachment."""
import sys,json
from pathlib import Path
import bpy
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from refinement_review import _tree
from render_slots import acquire,release

def inspect():
 line=bpy.data.objects['Initial lifting line'];tie=bpy.data.objects['Inferred upper fastening loop'];tree,_,_=_tree([tie]);center=sum((v.co for v in line.data.vertices[:8]),Vector())/8;cross=[]
 for edge in line.data.edges:
  if min(edge.vertices)>=24:continue
  a,b=[line.data.vertices[i].co for i in edge.vertices];delta=b-a
  if delta.length<1e-7:continue
  p,n,idx,d=tree.ray_cast(a,delta.normalized(),delta.length)
  if p is not None and 1e-6<d<delta.length-1e-6:cross.append(dict(edge=edge.index,point=list(p)))
 parity=[]
 for direction in [Vector((1,.237,.113)).normalized(),Vector((.157,1,.383)).normalized(),Vector((.421,.151,1)).normalized()]:
  current=center.copy();hits=0
  for _ in range(50):
   p,n,i,d=tree.ray_cast(current,direction,10000)
   if p is None:break
   hits+=1;current=p+direction*.0001
  parity.append(hits)
 return dict(line_endpoint=list(center),line_tip_edge_crossings=cross,tip_surface_crossing_count=len(cross),endpoint_parity_counts=parity,endpoint_inside_by_majority=sum(n%2 for n in parity)>=2,physically_connected=bool(cross)or sum(n%2 for n in parity)>=2)

def main():
 for k in ['00','01']:
  w=OUT/f'restart5-initial-nets/candidate-v5/profile-{k}';bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));d=inspect();write_json(w/'attachment-audit.json',dict(model_sha256=sha(w/'model.blend'),**d));print(k,d)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
