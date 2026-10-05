"""Read-only topology and native solid first-hit audit of the smooth root research."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json


def bvh(obj):
 obj.data.calc_loop_triangles()
 return BVHTree.FromPolygons([tuple(obj.matrix_world@v.co)for v in obj.data.vertices],[tuple(t.vertices)for t in obj.data.loop_triangles],all_triangles=True)


def main():
 variant=sys.argv[sys.argv.index('--')+1]if '--'in sys.argv else 'research-roots-v4'
 base=OUT/'restart3-tree06-root'/variant;model=base/'root.blend'
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();obj=next(o for o in bpy.context.scene.objects if o.type=='MESH')
 new=bvh(obj);bm=bmesh.new();bm.from_mesh(obj.data);nonmanifold=sum(not e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);remaining=set(bm.verts);components=[]
 while remaining:
  stack=[remaining.pop()];count=0
  while stack:
   v=stack.pop();count+=1
   for edge in v.link_edges:
    other=edge.other_vert(v)
    if other in remaining:remaining.remove(other);stack.append(other)
  components.append(count)
 bm.free()
 original=Path(json.loads((OUT/'restart3-tree06-root/probe.json').read_text())['model'])
 bpy.ops.wm.open_mainfile(filepath=str(original));bpy.context.view_layer.update()
 old=[bvh(o)for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-06' and 'wood 'in o.name]
 bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
 bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
 banks=[bvh(o)for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
 mask=np.asarray(Image.open(OUT/'baseline/masks/000006.png').convert('L'))>0
 def distance(tree,origin):
  hit,normal,face,d=tree.ray_cast(origin,-RAY,10000)
  return d if hit is not None else float('inf')
 box=(594,464,706,576);rows=[];counts=dict(new_first_hit_owned_wood=0,new_first_hit_outside_wood=0,old_wood_replaced=0,bank_replaced=0)
 for y in range(box[1],box[3]):
  for x in range(box[0],box[2]):
   origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;d=distance(new,origin)
   if not np.isfinite(d):continue
   old_distance=min(distance(tree,origin)for tree in old);bank_distance=min(distance(tree,origin)for tree in banks)
   if d>=min(old_distance,bank_distance):continue
   own=bool(0<=y-252<mask.shape[0] and 0<=x-517<mask.shape[1] and mask[y-252,x-517])
   previous='wood'if old_distance<bank_distance else 'bank'
   counts['new_first_hit_owned_wood'if own else 'new_first_hit_outside_wood']+=1;counts[previous+'_replaced'if previous=='bank'else 'old_wood_replaced']+=1
   rows.append(dict(pixel=[x,y],wood_domain=own,previous=previous,ray_depth_change=min(old_distance,bank_distance)-d))
 source=Image.open(OUT/'baseline/covered.png').convert('RGB').crop(box).resize((448,448),Image.Resampling.NEAREST);overlay=source.copy();draw=ImageDraw.Draw(overlay)
 for row in rows:
  x,y=row['pixel'];x=(x-box[0])*4;y=(y-box[1])*4
  if not row['wood_domain']:draw.rectangle((x,y,x+3,y+3),outline=(255,50,50))
 sheet=Image.new('RGB',(896,478),(45,45,45));sheet.paste(source,(0,0));sheet.paste(overlay,(448,0));ImageDraw.Draw(sheet).text((5,454),'Original source / red=new solid first hits outside native wood6',fill='white');sheet.save(base/'full-neighborhood-overreach.png')
 write_json(base/'full-neighborhood-audit.json',dict(model_sha256=sha(model),original_model_sha256=sha(original),bank_sha256=sha(bank),root_topology=dict(nonmanifold_edges=nonmanifold,connected_components=components,signed_volume=volume),crop=box,counts=counts,pixels=rows,limitations=['Solid first-hit geometry diagnostic only; no source RGBA or alpha shader parity claim.','Context is the three approved wood receivers and exact bank, not the whole scene or neighboring crowns.','Outside-native-wood count is an ownership warning, not authority to map painted ground onto wood.']))
 print(counts,flush=True)


if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
