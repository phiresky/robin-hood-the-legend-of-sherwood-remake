"""Review saved canopy motion with complete geometry framing and fixed cameras."""
import sys,json,hashlib
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector,Matrix
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from tree_geometry import RAY,SIN
from mathutils.bvhtree import BVHTree
OUT=ROOT/'level-editor/work/croisement02-refinement';BASE=OUT/'restart14-canopy-animation/tree42-motion-v1';DEST=BASE/'review-v3';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 DEST.mkdir(exist_ok=False);r=json.loads((BASE/'report.json').read_text());assert sha(BASE/'prototype.blend')==r['prototype_sha256'];bpy.ops.wm.open_mainfile(filepath=str(BASE/'prototype.blend'));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-42'];crown=next(o for o in objects if o.get('projection_component')=='crown');views=json.loads((OUT/'restart2-canopy/soft-envelope-packaged-v1/assets/croisement02-tree-42/modified/views.json').read_text())['views'];source=json.loads((OUT/'restart14-canopy-animation/source-reconciliation-v1/report.json').read_text())['groups'][1];support=[]
 for phase,f in enumerate(source['frames']):
  scene.frame_set(1+phase*4);bpy.context.view_layer.update();ev=crown.evaluated_get(bpy.context.evaluated_depsgraph_get());mesh=ev.to_mesh();mesh.calc_loop_triangles();tree=BVHTree.FromPolygons([ev.matrix_world@v.co for v in mesh.vertices],[list(t.vertices)for t in mesh.loop_triangles],all_triangles=True);a=np.array(Image.open(f['path']));x,y,_,_=f['bbox'];missing=[]
  for py,px in zip(*np.nonzero(a[:,:,3])):
   sx=float(px+x)+.5;sy=float(py+y)+.5;hit=tree.ray_cast(Vector((sx,-sy/SIN,0))+RAY*5000,-RAY)[0]
   if hit is None:missing.append([int(sx),int(sy)])
  ev.to_mesh_clear();support.append({'phase':phase,'source_pixels':int(np.count_nonzero(a[:,:,3])),'solid_motion_misses':missing})
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in objects
 scene.frame_set(29);bpy.context.view_layer.update();points=[]
 for o in objects:
  ev=o.evaluated_get(bpy.context.evaluated_depsgraph_get());mesh=ev.to_mesh();points.extend(ev.matrix_world@v.co for v in mesh.vertices);ev.to_mesh_clear()
 scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';data=bpy.data.cameras.new('Review camera');data.type='ORTHO';data.clip_end=20000;cam=bpy.data.objects.new('Review camera',data);scene.collection.objects.link(cam);scene.camera=cam
 cameras=[]
 for v in views:
  matrix=Matrix(v['camera_matrix_world']);right=matrix.to_3x3()@Vector((1,0,0));up=matrix.to_3x3()@Vector((0,1,0));coords=np.array([[(p-matrix.translation).dot(right),(p-matrix.translation).dot(up)]for p in points]);mid=(coords.min(0)+coords.max(0))/2;matrix.translation+=right*float(mid[0])+up*float(mid[1]);scale=float(np.max(np.ptp(coords,axis=0))*1.12);cameras.append((matrix,scale))
 images=[]
 for phase,indices in [(0,[0]),(7,range(8)),(14,[0])]:
  scene.frame_set(1+phase*4)
  for i in indices:
   cam.matrix_world,data.ortho_scale=cameras[i];p=DEST/f'phase-{phase:02}-view-{i}.png';scene.render.filepath=str(p);bpy.ops.render.render(write_still=True);images.append({'phase':phase,'view':i,'sha256':sha(p),'path':str(p)})
 sheet=Image.new('RGB',(1536,808),'#ddd');draw=ImageDraw.Draw(sheet)
 for i in range(8):
  im=Image.open(DEST/f'phase-07-view-{i}.png');x=i%4*384;y=i//4*404;sheet.paste(im,(x,y+20),im);draw.text((x+5,y+4),f'Phase7 actual view{i}',fill='black')
 sheet.save(DEST/'actual-eight.png');assert np.array_equal(np.array(Image.open(DEST/'phase-00-view-0.png')),np.array(Image.open(DEST/'phase-14-view-0.png')))
 (DEST/'report.json').write_text(json.dumps({'status':'RENDERED_FOR_SELF_REVIEW','prototype_sha256':r['prototype_sha256'],'source_report_sha256':sha(BASE/'report.json'),'per_phase_solid_support':support,'images':images,'fixed_cameras':[{'matrix':[list(row)for row in m],'ortho_scale':s}for m,s in cameras],'loop_exact':True,'native_view0_preserved_direction':True,'scope':'Full geometry framing. Solid support is an upper bound, not alpha or adjacent first-hit authority.'},indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
