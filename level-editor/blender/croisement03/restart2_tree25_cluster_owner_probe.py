"""Inspect the few native classification differences without changing the model."""
import json,math,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release

def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment/cluster-geometry-v2';acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(e/'worker.blend'));o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles);tree=BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True);atlas={}
  for slot,mat in enumerate(m.materials):
   if not mat or not mat.get('foliage_physical_opacity'):continue
   node=next(n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image);a=np.empty(len(node.image.pixels),np.float32);node.image.pixels.foreach_get(a);atlas[slot]=(a.reshape(node.image.size[1],node.image.size[0],4),m.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map],mat.get('foliage_card_sides')=='paired-one-sided')
  ray=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))));rows=[]
  for x,y in [(1405,722),(1404,740),(1406,750),(1390,722)]:
   origin=Vector((x+.5,-(y+.5)/math.sin(math.radians(35)),0))+ray*10000;hits=[]
   for step in range(128):
    p,n,tid,d=tree.ray_cast(origin,-ray)
    if p is None:break
    tri=ts[tid];f=m.polygons[tri.polygon_index];slot=f.material_index
    if slot not in atlas: hits.append(dict(slot=slot,wood=True));break
    a,uv,sided=atlas[slot];mapped=barycentric_transform(p,*[vs[i] for i in tri.vertices],*[Vector((*uv.data[i].uv,0)) for i in tri.loops]);tx=min(a.shape[1]-1,int((mapped.x%1)*a.shape[1]));ty=min(a.shape[0]-1,int((mapped.y%1)*a.shape[0]));back=sided and n.dot(-ray)>=0
    hits.append(dict(slot=slot,face=f.index,point=list(p),alpha=float(a[ty,tx,3]),pixel=[tx,ty],uv=list(mapped),backfacing=back,ownership=[m.color_attributes['Source ownership'].data[i].color[0] for i in tri.loops]))
    origin=p-ray*.002
   rows.append(dict(pixel=[x,y],hits=hits))
  (e/'owner-probe.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps(rows))
 finally:release()
if __name__=='__main__':main()
