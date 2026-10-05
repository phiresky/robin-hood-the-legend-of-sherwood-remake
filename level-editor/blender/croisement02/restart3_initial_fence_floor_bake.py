"""Apply the approved initial fence floor domain without rolling back other ground fixes."""
import sys,json,hashlib,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
from restart3_initial_fence_contact import link
from review_bank_candidate import camera
from tree_geometry import SIN,COS,RAY
E=OUT/'restart3-initial-fence/floor-fill-v1'
G=E/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
BASE=OUT/'restart2-state/trap-ground-continuation-model-v1/model.blend'
FENCE=OUT/'restart3-initial-fence/geometry-v6/model.blend'
PATCH=OUT/'restart3-fence-receiver/terminal-v3/model.blend'
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def main():
 out=E/'bake-v1';assert not out.exists()
 assert sha(BASE)=='fe8da24ebb5f696f8df6453baf2a82454737bf4157f1a65ecdf78b2380d8e7ec'
 assert sha(FENCE)=='46579f5398d1495b13e8d8433fa1a0687be447e025cd9ba745f5d7251076c5a0'
 patchhash=sha(PATCH);a=json.loads((E/'approval.json').read_text());review=json.loads((E/'visual-review.json').read_text())
 assert a['status']=='approved' and a['approved_by']=='user' and sha(E/'input.png')==a['input_sha256'] and sha(E/'mask.png')==a['mask_sha256']
 assert review['status']=='PASS for guarded bake' and review['generated_sha256']==sha(G/'generated-preserved.png')
 original=rgba(E/'input.png');raw=rgba(G/'generated-raw.png');preserved=rgba(G/'generated-preserved.png');mask=rgba(E/'mask.png')[:,:,3]==0
 assert original.shape==raw.shape==preserved.shape==(1152,1792,4) and mask.sum()==5419
 expected=original.copy();expected[mask,:3]=raw[mask,:3];assert np.array_equal(expected,preserved)
 known=np.array(Image.open(OUT/'restart2-ground-completion/preparation-v1/known.png').convert('L'))>0
 assert known.sum()==772189 and not (known&mask).any()
 bpy.ops.wm.open_mainfile(filepath=str(BASE));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update()
 ground=bpy.data.objects['Croisement02 Terrain'];signature=geometry(ground);node,current=atlas(ground)
 changed=np.any(current!=original,axis=2);assert changed.sum()==14 and not (changed&mask).any()
 filled=current.copy();filled[mask,:3]=raw[mask,:3];assert np.array_equal(filled[~mask],current[~mask]) and np.array_equal(filled[known],current[known])
 assert np.array_equal(filled[:,:,3],current[:,:,3]);out.mkdir();Image.fromarray(current).save(out/'current-approved-atlas.png');Image.fromarray(filled).save(out/'composite.png')
 image=bpy.data.images.load(str(out/'composite.png'),check_existing=False);image.pack();node.image=image
 bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
 bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));bpy.context.view_layer.update();ground=bpy.data.objects['Croisement02 Terrain']
 assert geometry(ground)==signature and np.array_equal(atlas(ground)[1],filled)
 write_json(out/'validation.json',dict(status='PASS',model_sha256=sha(out/'model.blend'),base_model_sha256=sha(BASE),input_approval_sha256=sha(E/'approval.json'),geometry_uv_signature=signature,geometry_uv_unchanged=True,packed_rgba_exact=True,editable_pixels=5419,inferred_only=True,native_returns=0,protected_known_pixels=772189,all_outside_pixels_preserved=int((~mask).sum()),prior_approved_14_pixels_preserved=True,alpha_exact=True,applied_terminal_model_sha256=patchhash,applied_terminal_unchanged=True,mask_sha256=sha(E/'mask.png'),generated_sha256=sha(G/'generated-preserved.png'),appearance_user_approval=None))
 scene=bpy.data.scenes.new('Initial fence inferred floor contact');bpy.context.window.scene=scene;link(scene,ground);ground.hide_render=False
 with bpy.data.libraries.load(str(FENCE),link=False) as(src,dst):dst.objects=[n for n in src.objects if 'South Field Wattle Fence part' in n]
 meshes=[]
 for obj in dst.objects:
  if obj and obj.type=='MESH':link(scene,obj);obj.hide_render=False;meshes.append(obj)
 bpy.context.view_layer.update();assert geometry(ground)==signature and len(meshes)==2
 signatures={o.name:geometry(o)for o in meshes}
 target=Vector((1094,-887/SIN,0));sheet=Image.new('RGB',(1408,512),'#303030')
 for index,(name,pixels) in enumerate([('baseline',current),('candidate',filled)]):
  packed=bpy.data.images.load(str(out/('current-approved-atlas.png' if name=='baseline' else 'composite.png')),check_existing=False);node=atlas(ground)[0];node.image=packed
  camera(scene,target,RAY,704,512,220);scene.render.filepath=str(out/(name+'-native.png'));bpy.ops.render.render(write_still=True,scene=scene.name)
  sheet.paste(Image.open(scene.render.filepath).convert('RGB'),(index*704,0))
 sheet.save(out/'native-before-after.png')
 sheet=Image.new('RGB',(1408,512),'#303030')
 for i in range(8):
  angle=i*math.pi/4;direction=RAY if i==0 else Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN))
  camera(scene,Vector((1094,-1555,15)),direction,704,512,245);scene.render.filepath=str(out/f'view-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
  im=Image.open(scene.render.filepath).convert('RGB').resize((352,256));sheet.paste(im,(i%4*352,i//4*256))
 sheet.save(out/'actual8.png')
 for i,direction in enumerate([Vector((.45,-.65,.55)).normalized(),Vector((-.55,.7,.45)).normalized()]):
  camera(scene,Vector((1094,-1555,25)),direction,704,512,255);scene.render.filepath=str(out/f'contact-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
 assert geometry(ground)==signature and signatures=={o.name:geometry(o)for o in meshes} and sha(PATCH)==patchhash
 write_json(out/'contact-validation.json',dict(status='PASS',native_camera_first=True,ground_transform_exact=True,fence_geometry_signatures=signatures,fence_model_sha256=sha(FENCE),saved_model_contains_ground_only=True,no_context_model_saved=True))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
