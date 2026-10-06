"""Full-asset cap candidate review and exact retained-receiver/source guard."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from restart3_tree06_root_correction import fingerprint
from restart3_tree06_root_review import configure
from restart2_sign_neighbors import camera_to,render
from restart7_wall101_cap_candidate import BASE,D,ASSET
GEOMETRY_EVIDENCE=D

def main():
 dest=D/'full-asset-review-v1';dest.mkdir(exist_ok=False);mask=np.asarray(Image.open(OUT/'baseline/masks/000101.png').convert('L'))>0;yy,xx=np.nonzero(mask);pixels=list(zip(xx+1419,yy+829));records=[]
 for path in [BASE,D/'model.blend']:
  bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET];trees=[]
  for o in objects:
   o.data.calc_loop_triangles();trees.append(BVHTree.FromPolygons([tuple(o.matrix_world@v.co)for v in o.data.vertices],[tuple(t.vertices)for t in o.data.loop_triangles],all_triangles=True))
  coverage=[]
  for x,y in pixels:
   origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0))+RAY*5000;coverage.append(any(t.ray_cast(origin,-RAY,10000)[0]is not None for t in trees))
  records.append(dict(model_sha256=sha(path),other_receivers={o.name:fingerprint(o)for o in objects if o.get('source_node')!='building-010'},coverage=coverage))
 assert records[0]['other_receivers']==records[1]['other_receivers'];old,new=[np.asarray(r['coverage'])for r in records];lost=[list(map(int,pixels[i]))for i in np.flatnonzero(old&~new)];gained=[list(map(int,pixels[i]))for i in np.flatnonzero(new&~old)];assert not lost
 scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in objects
 points=np.array([tuple(o.matrix_world@v.co)for o in objects for v in o.data.vertices]);center=Vector((points.min(0)+points.max(0))/2);scale=float(np.linalg.norm(points-np.asarray(center),axis=1).max()*2.2);camera=configure(scene);scene.render.resolution_x=512;scene.render.resolution_y=512;camera.data.ortho_scale=scale
 world=bpy.data.worlds.new('Full wall review world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world;light=bpy.data.lights.new('Full wall solid sun','SUN');light.energy=2;sun=bpy.data.objects.new(light.name,light);scene.collection.objects.link(sun);sun.rotation_euler=(.5,-.6,-.4)
 solid=bpy.data.materials.new('Full wall neutral');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.45,.45,.45,1)
 for mode in ['actual','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None;sheet=Image.new('RGB',(2048,1088),(45,45,45))
  for i in range(8):
   a=i*math.pi/4;camera_to(camera,center,Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN)));pic=render(scene,dest/f'{mode}-{i}.png');sheet.paste(pic,(i%4*512,i//4*544),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i%4*512+5,i//4*544+516),f'Full asset {mode} {i}; native first',fill='white')
  sheet.save(dest/f'{mode}-eight.png')
 write_json(dest/'receipt.json',dict(model_sha256=sha(D/'model.blend'),base_sha256=sha(BASE),all_six_other_receivers_exact=True,retained_receiver_fingerprints=records[0]['other_receivers'],native_mask101_sha256=sha(OUT/'baseline/masks/000101.png'),native_domain_pixels=len(pixels),old_covered=int(old.sum()),new_covered=int(new.sum()),gained_pixels=gained,lost_pixels=lost,original_camera_first=True,full_asset_camera=dict(center=list(center),ortho_scale=scale,resolution=[512,512]),original_geometry_not_resaved=True,scope='All seven receivers, full native101 solid source-domain guard and full unclipped eight-view actual/solid supplement. Changed cap topology guard remains separately hash-bound.',files={str(p):sha(p)for p in [dest/'actual-eight.png',dest/'solid-eight.png',GEOMETRY_EVIDENCE/'source-audit.json',GEOMETRY_EVIDENCE/'changes.json']}));print('FULL GUARD',len(pixels),'gains',len(gained),'loss',len(lost),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
