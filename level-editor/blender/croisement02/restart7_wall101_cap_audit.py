"""Compare native cap first hits and unchanged receiver topology on the saved candidate."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from restart7_wall101_cap_candidate import BASE,D,ASSET

def main():
 records=[];box=(1530,835,1555,855);pixels=[(x,y)for y in range(box[1],box[3])for x in range(box[0],box[2])];target={(1541,844),(1542,844),(1543,844),(1544,844),(1544,845)}
 native=np.asarray(Image.open(OUT/'baseline/masks/000101.png').convert('L'))>0
 for path in [BASE,D/'model.blend']:
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();trees=[];topology=[]
  for o in bpy.context.scene.objects:
   if o.type!='MESH'or o.get('asset_group')!=ASSET:continue
   o.data.calc_loop_triangles();t=BVHTree.FromPolygons([tuple(o.matrix_world@v.co)for v in o.data.vertices],[tuple(t.vertices)for t in o.data.loop_triangles],all_triangles=True);trees.append(t);bm=bmesh.new();bm.from_mesh(o.data);topology.append(dict(object=o.name,vertices=len(bm.verts),faces=len(bm.faces),nonmanifold=sum(not e.is_manifold for e in bm.edges),zero_area=sum(f.calc_area()<1e-9 for f in bm.faces)));bm.free()
  covered=[]
  for x,y in pixels:
   origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000
   if any(t.ray_cast(origin,-RAY,10000)[0]is not None for t in trees):covered.append((x,y))
  records.append(dict(sha256=sha(path),topology=topology,covered=covered))
 old,new=[set(map(tuple,r['covered']))for r in records];gain=new-old;loss=old-new;outside=[p for p in gain if not native[p[1]-829,p[0]-1419]]
 report=dict(model_sha256=sha(D/'model.blend'),base_sha256=sha(BASE),target_five_before=len(old&target),target_five_after=len(new&target),target_missing=sorted(target-new),new_coverage=sorted(gain),lost_coverage=sorted(loss),new_coverage_outside_native101=outside,topology_unchanged=records[0]['topology']==records[1]['topology'],topology=records[1]['topology'],crop=box,limitations=['Solid first hits; actual material eight-view/source comparison is separate.','Native101 ownership alone does not prove any outside-target gain is semantically masonry; source overlay remains required.'])
 write_json(D/'source-audit.json',report);src=Image.open(OUT/'animation-references/composite-frame-0.png').crop(box).resize((500,400),Image.Resampling.NEAREST);dr=ImageDraw.Draw(src)
 for x,y in gain:dr.rectangle(((x-box[0])*20,(y-box[1])*20,(x-box[0])*20+19,(y-box[1])*20+19),outline=(255,40,40)if(x,y)in outside else(20,220,255),width=2)
 src.save(D/'source-gain-overlay.png');print({k:report[k]for k in ['target_five_after','target_missing','new_coverage_outside_native101','topology_unchanged']})
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
