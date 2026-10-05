"""Expose root contact from eight cameras without crown occlusion in review only."""
import sys,math,json
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS
from render_slots import acquire,release
from restart3_tree06_root_review import configure
from restart2_sign_neighbors import camera_to,render
from sign_context_import import append_verified
from evidence_io import sha,write_json


def main(variant='collar-v3',before=False):
 base=OUT/'restart3-tree06-root'/variant;dest=base/('root-close-before'if before else 'root-close');dest.mkdir(exist_ok=False)
 model=Path(json.loads((base/'report.json').read_text())['input_model'])if before else base/'model.blend'
 bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
 bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
 names=[o.name for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
 expected={n:dict(matrix_world=[list(r)for r in bpy.data.objects[n].matrix_world])for n in names}
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();scene=bpy.context.scene
 hidden=[]
 for o in scene.objects:
  if o.type=='MESH':
   o.hide_render=o.get('asset_group')!='croisement02-tree-06' or o.get('projection_component')=='crown' or o.name.endswith('/ Crown')
   if o.hide_render:hidden.append(o.name)
 banks,receipt=append_verified(scene,bank,names,expected)
 camera=configure(scene);camera.data.ortho_scale=125
 center=Vector((650,(-535-COS*47)/SIN,47));sheet=Image.new('RGB',(1536,816),(65,65,65))
 for i in range(8):
  angle=i*math.pi/4;camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)))
  pic=render(scene,dest/f'contact-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'))
  ImageDraw.Draw(sheet).text((i%4*384+5,i//4*408+388),f'Root contact {i}; crown hidden for inspection',fill='white')
 sheet.save(dest/'contact-eight.png')
 if before:
  write_json(dest/'report.json',dict(model_sha256=sha(model),bank_sha256=sha(bank),verified_context=receipt,hidden_in_review_only=hidden,native_camera_first=True,no_model_saved=True,status='Unchanged approved baseline comparison'))
  return
 root=scene.objects['Northwest Tree 06 / Root collar continuation']
 bm=bmesh.new();bm.from_mesh(root.data);nonmanifold=sum(not e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True)
 remaining=set(bm.verts);components=[]
 while remaining:
  stack=[remaining.pop()];count=0
  while stack:
   v=stack.pop();count+=1
   for edge in v.link_edges:
    other=edge.other_vert(v)
    if other in remaining:remaining.remove(other);stack.append(other)
  components.append(count)
 bm.free();trees=[]
 for obj in banks:
  obj.data.calc_loop_triangles();trees.append(BVHTree.FromPolygons([tuple(obj.matrix_world@v.co)for v in obj.data.vertices],[tuple(t.vertices)for t in obj.data.loop_triangles],all_triangles=True))
 clearances=[];missing=[]
 for v in root.data.vertices:
  p=root.matrix_world@v.co
  if p.z>42.51:continue
  hits=[tree.ray_cast(Vector((p.x,p.y,2000)),Vector((0,0,-1)),4000)[0]for tree in trees]
  heights=[hit.z for hit in hits if hit is not None]
  if heights:clearances.append(max(heights)-p.z)
  else:missing.append(list(p))
 physics=dict(nonmanifold_edges=nonmanifold,connected_components=components,signed_volume=volume,buried_vertex_samples=len(clearances),missing_bank_below_vertices=missing,minimum_buried_vertical_clearance=min(clearances)if clearances else None)
 write_json(dest/'report.json',dict(model_sha256=sha(base/'model.blend'),bank_sha256=sha(bank),verified_context=receipt,hidden_in_review_only=hidden,native_camera_first=True,no_model_saved=True,root_physics=physics))


if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'collar-v3',before='--before' in sys.argv)
 finally:release()
