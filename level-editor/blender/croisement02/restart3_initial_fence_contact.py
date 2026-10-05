"""Reopen candidate fence and verify unchanged ground in source and oblique contact."""
import sys,json,math,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
from review_bank_candidate import camera
from tree_geometry import SIN,COS,RAY
D=OUT/'restart3-initial-fence/geometry-v6';GROUND=OUT/'restart2-ground-completion/approved-fill-retry-v2/bake-v1/model.blend';BASE=OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment/bake-v1/worker.blend'

def link(scene,obj):
 if obj.name not in scene.objects:scene.collection.objects.link(obj)
 parent=obj.parent
 while parent:
  if parent.name not in scene.objects:scene.collection.objects.link(parent)
  parent=parent.parent

def main():
 out=D/'contact-v1';out.mkdir(exist_ok=False)
 assert sha(GROUND)=='16c638be71eeb76e86439a0fdb14bac1e7bb9562afe20d175b58d0df96fb4ec2'
 bpy.ops.wm.open_mainfile(filepath=str(GROUND));bpy.context.view_layer.update();ground=bpy.data.objects['Croisement02 Terrain'];sig=geometry(ground);original=atlas(ground)[1]
 result=[];frozen={};outside=[];original_images={}
 for label,model in [('baseline',BASE),('candidate',D/'model.blend')]:
  bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-south-field-wattle-fence'];signatures={o.name:geometry(o)for o in objects}
  images={n.image for o in objects for m in o.data.materials if m and m.use_nodes for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image};packed={im.name:hashlib.sha256(im.packed_file.data).hexdigest()for im in images if im.packed_file}
  if label=='baseline':original_images=packed
  else:
   assert all(packed.get(n)==h for n,h in original_images.items());assert sha(OUT/'animation-references/composite-frame-0.png')in packed.values()
  for obj in objects:
   world=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);uv=np.array([v.uv[:] for v in obj.data.uv_layers[0].data]);faces=[list(p.vertices)for p in obj.data.polygons]
   if label=='baseline':frozen[obj.name]=(world,uv,faces,[p.material_index for p in obj.data.polygons])
   else:
    old,olduv,oldfaces,oldmat=frozen[obj.name];protected=(old[:,0]<=1018)|(old[:,0]>=1170);assert np.array_equal(world[protected],old[protected]);assert np.array_equal(uv,olduv);assert faces==oldfaces
    protectedfaces=[i for i,f in enumerate(faces)if all(old[v,0]<=1018 for v in f)or all(old[v,0]>=1170 for v in f)];assert all(obj.data.polygons[i].material_index==oldmat[i] for i in protectedfaces);outside.append(dict(object=obj.name,protected_vertices=int(protected.sum()),protected_faces=len(protectedfaces),outside_geometry_exact=True,all_original_UV_exact=True,outside_material_assignment_exact=True))
  scene=bpy.data.scenes.new('Initial fence grounded '+label);bpy.context.window.scene=scene
  for obj in objects:link(scene,obj);obj.hide_render=False
  with bpy.data.libraries.load(str(GROUND),link=False)as(src,dst):dst.objects=['Croisement02 Terrain']
  ground=dst.objects[0];link(scene,ground);ground.hide_render=False;bpy.context.view_layer.update()
  assert geometry(ground)==sig and np.array_equal(atlas(ground)[1],original)
  assert signatures=={o.name:geometry(o)for o in objects}
  target=Vector((1094,-887/SIN,0));camera(scene,target,RAY,704,512,220);scene.render.filepath=str(out/(label+'-native.png'));bpy.ops.render.render(write_still=True,scene=scene.name)
  if label=='baseline':continue
  contacts=[]
  for obj in objects:
   bm=bmesh.new();bm.from_mesh(obj.data);topology=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),zero_area=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
   points=[obj.matrix_world@v.co for v in obj.data.vertices];contacts.append(dict(object=obj.name,min_z=min(p.z for p in points),max_z=max(p.z for p in points),topology=topology))
  for i,direction in enumerate([Vector((.45,-.65,.55)).normalized(),Vector((-.55,.7,.45)).normalized()]):
   camera(scene,Vector((1094,-1555,25)),direction,704,512,255);scene.render.filepath=str(out/f'oblique-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
  ground.hide_render=True;solid=bpy.data.materials.new('Initial fence solid inspection');solid.use_nodes=True;solid.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.45,.45,.45,1);scene.view_layers[0].material_override=solid
  world=bpy.data.worlds.new('Review ambient');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.7;scene.world=world
  light=bpy.data.lights.new('Review sun','SUN');light.energy=2;sun=bpy.data.objects.new('Review sun',light);scene.collection.objects.link(sun);sun.rotation_euler=(.5,-.6,-.4)
  points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];center=sum(points,Vector())/len(points);sheet=Image.new('RGB',(1280,384),'#303030')
  for i in range(8):
   angle=i*math.pi/4;direction=RAY if i==0 else Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(scene,center,direction,640,384,760);scene.render.filepath=str(out/f'solid-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
   im=Image.open(scene.render.filepath).convert('RGBA');bg=Image.new('RGBA',im.size,'#303030');bg.alpha_composite(im);sheet.paste(bg.convert('RGB').resize((320,192)),(i%4*320,i//4*192))
  sheet.save(out/'solid8.png');result=contacts
 source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB');crop=(984,807,1204,967);sheet=Image.new('RGB',(1056,300),'#303030');draw=ImageDraw.Draw(sheet)
 for i,(label,image)in enumerate([('Original source',source.crop(crop).resize((352,256),Image.Resampling.NEAREST)),('Approved fence / unchanged ground',Image.open(out/'baseline-native.png').resize((352,256))),('Candidate / unchanged ground',Image.open(out/'candidate-native.png').resize((352,256)))]):sheet.paste(image.convert('RGB'),(i*352,32));draw.text((i*352+4,8),label,fill='white')
 sheet.save(out/'source-baseline-candidate.png')
 write_json(out/'validation.json',dict(status='Reopened scoped contact proof; visual review pending',model_sha256=sha(D/'model.blend'),ground_model_sha256=sha(GROUND),ground_geometry_uv_signature=sig,ground_RGBA_exact=True,source_camera_first=True,imported_transforms_exact=True,contacts=result,original_material_images_exact=original_images,native_source_png_exact=True,outside_initial_geometry_preserved=outside,applied_state_artifact_sha256=sha(OUT/'fence-state-candidate-v2/worker.blend'),no_model_saved=True))
 print(result,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
