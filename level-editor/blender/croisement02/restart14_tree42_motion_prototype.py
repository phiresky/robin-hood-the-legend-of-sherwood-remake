"""Private, reversible canopy deformation from conservative native patch motion."""
import sys,json,hashlib,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from refinement_workspace import _geometry
from tree_geometry import SIN,COS,RAY
OUT=ROOT/'level-editor/work/croisement02-refinement';BASE=OUT/'restart2-textures/approved6-canopy-fill-v1/croisement02-tree-42/native-front-preparation-v2/experiment/bake-v1/worker.blend';DEST=OUT/'restart14-canopy-animation/tree42-motion-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 DEST.mkdir(exist_ok=True);assert sha(BASE)=='8c89650bc54509823ece450fa41109bfbf19f891743bb556175fd02928b849e5'
 source=json.loads((OUT/'restart14-canopy-animation/source-reconciliation-v1/report.json').read_text())['groups'][1];motion=json.loads((OUT/'restart14-canopy-animation/tree42-correspondence-v1/report.json').read_text())
 bpy.ops.wm.open_mainfile(filepath=str(BASE));views=json.loads((OUT/'restart2-canopy/soft-envelope-packaged-v1/assets/croisement02-tree-42/modified/views.json').read_text());scene=bpy.data.scenes[views['scene_name']];bpy.context.window.scene=scene;objects=[scene.objects[n]for n in views['object_names']];crown=next(o for o in objects if o.get('projection_component')=='crown');guards={o.name:_geometry(o,protect_appearance=True)for o in objects};world=np.array([crown.matrix_world@v.co for v in crown.data.vertices]);native=np.stack([world[:,0],-world[:,1]*SIN-world[:,2]*COS],axis=1)
 crown.data.calc_loop_triangles();triangles=[tuple(t.vertices)for t in crown.data.loop_triangles];tree=BVHTree.FromPolygons(world.tolist(),triangles,all_triangles=True);support=[]
 for f in source['frames']:
  a=np.array(Image.open(f['path']).convert('RGBA'));x,y,_,_=f['bbox'];hit=0;missing=[]
  for py,px in zip(*np.nonzero(a[:,:,3])):
   sx=float(px+x)+.5;sy=float(py+y)+.5;origin=Vector((sx,-sy/SIN,0))+RAY*5000;point,_,_,_=tree.ray_cast(origin,-RAY)
   if point is None:missing.append([int(sx),int(sy)])
   else:hit+=1
  support.append({'phase':len(support),'source_pixels':int(np.count_nonzero(a[:,:,3])),'solid_crown_hits':hit,'missing':missing})
 crown.shape_key_add(name='Approved static basis');inverse=crown.matrix_world.inverted().to_3x3();phase_stats=[]
 for row in motion['rows'][1:]:
  samples=[s for s in row['samples']if s['accepted']];points=np.array([s['pixel']for s in samples]);vectors=np.array([s['delta']for s in samples]);delta=np.zeros((len(world),2))
  for start in range(0,len(world),2000):
   dist=np.sum((native[start:start+2000,None,:]-points[None,:,:])**2,axis=2);idx=np.argsort(dist,axis=1)[:,:4];ds=np.take_along_axis(dist,idx,axis=1);weight=np.exp(-ds/(2*12**2))*(ds<28**2);den=weight.sum(axis=1);delta[start:start+2000]=np.sum(vectors[idx]*weight[:,:,None],axis=1)/np.maximum(den[:,None],1e-9)
  displacement=np.stack([delta[:,0],-delta[:,1]*SIN,-delta[:,1]*COS],axis=1);key=crown.shape_key_add(name=f'Native-supported motion {row["phase"]:02}');local=np.array([inverse@Vector(v)for v in displacement]);coordinates=np.array([v.co[:]for v in crown.data.vertices])+local;key.data.foreach_set('co',coordinates.reshape(-1));phase_stats.append({'phase':row['phase'],'moving_vertices':int(np.count_nonzero(np.linalg.norm(displacement,axis=1)>1e-5)),'maximum_displacement':float(np.max(np.linalg.norm(displacement,axis=1))),'ray_depth_change_max':float(np.max(np.abs(displacement@np.array(RAY))))})
 for phase in range(15):
  for i,key in enumerate(crown.data.shape_keys.key_blocks[1:],1):key.value=float(i==phase%14);key.keyframe_insert('value',frame=1+phase*4)
 action=crown.data.shape_keys.animation_data.action
 for layer in action.layers:
  for strip in layer.strips:
   for slot in action.slots:
    bag=strip.channelbag(slot)
    if bag:
     for curve in bag.fcurves:
      for key in curve.keyframe_points:key.interpolation='CONSTANT'
 scene.render.fps=25;scene.frame_start=1;scene.frame_end=56;scene.frame_set(1);bpy.context.view_layer.update();assert guards=={o.name:_geometry(o,protect_appearance=True)for o in objects}
 for other in list(bpy.data.scenes):
  if other!=scene:bpy.data.scenes.remove(other)
 crown_name=crown.name
 if not (DEST/'prototype.blend').exists():bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'prototype.blend'))
 bpy.ops.wm.open_mainfile(filepath=str(DEST/'prototype.blend'));scene=bpy.context.scene;crown=scene.objects[crown_name];scene.frame_set(1);assert guards=={name:_geometry(scene.objects[name],protect_appearance=True)for name in guards}
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.name not in guards
 scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';camdata=bpy.data.cameras.new('Prototype camera');camdata.type='ORTHO';camdata.clip_end=20000;cam=bpy.data.objects.new('Prototype camera',camdata);scene.collection.objects.link(cam);scene.camera=cam
 images=[]
 for phase,indices in [(0,[0]),(7,range(8)),(14,[0])]:
  scene.frame_set(1+phase*4)
  for i in indices:
   v=views['views'][i];cam.matrix_world=Matrix(v['camera_matrix_world']);camdata.ortho_scale=v['ortho_scale'];p=DEST/f'phase-{phase:02}-view-{i}.png';scene.render.filepath=str(p);bpy.ops.render.render(write_still=True);images.append({'phase':phase,'view':i,'path':str(p),'sha256':sha(p)})
 sheet=Image.new('RGB',(1536,808),'#ddd');draw=ImageDraw.Draw(sheet)
 for i in range(8):
  p=DEST/f'phase-07-view-{i}.png';im=Image.open(p);x=i%4*384;y=i//4*404;sheet.paste(im,(x,y+20),im);draw.text((x+5,y+4),f'Phase7 actual view{i}',fill='black')
 sheet.save(DEST/'actual-eight.png');a=np.array(Image.open(DEST/'phase-00-view-0.png'));b=np.array(Image.open(DEST/'phase-14-view-0.png'));assert np.array_equal(a,b)
 report={'status':'PRIVATE_PROTOTYPE_REQUIRES_VISUAL_REVIEW','source_worker_sha256':sha(BASE),'prototype_sha256':sha(DEST/'prototype.blend'),'static_basis_appearance_uv_geometry_exact':True,'wood_fixed':True,'loop_native_render_exact':True,'phase0_solid_support':support,'phase_motion':phase_stats,'images':images,'limitations':['Solid triangle ray hits are an upper bound; material alpha and adjacent static first-hit ownership are not yet certified.','Measured motion is image-plane patch displacement; continuous interpolation across triangles and hidden/rear deformation are inferred.','All material and UV values remain unchanged; no phase texture swaps or billboards.','No renderer/runtime/catalog integration; this is one local geometry-motion prototype, not all44 completion.']};(DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('PROTOTYPE COMPLETE',sha(DEST/'report.json'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
