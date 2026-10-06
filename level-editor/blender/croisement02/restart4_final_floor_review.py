"""Review reopened final floor in eight directions and unchanged bank contact."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
from restart3_initial_fence_contact import link
from review_bank_candidate import camera
from tree_geometry import SIN,RAY
D=OUT/'restart4-final-floor-bake-v1';BANK=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
def main():
 out=D/'saved-review';out.mkdir(exist_ok=False);assert sha(D/'model.blend')=='206c7c562b9aa9fc204ce42c0b0e9b1b78a22bee79ac00e6dab7fbbc59c64630';assert sha(BANK)=='69ecb7b704e30d6d64565a44aa810a21b924195609dbe7ac35818a0209137641'
 bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));scene=bpy.data.scenes.new('Saved final floor');bpy.context.window.scene=scene;ground=bpy.data.objects['Croisement02 Terrain'];link(scene,ground);ground.hide_render=False;bpy.context.view_layer.update();sig=geometry(ground);node,pixels=atlas(ground);assert np.array_equal(pixels,np.array(Image.open(D/'composite.png').convert('RGBA')));candidate_image=node.image
 points=[ground.matrix_world@Vector(v) for v in ground.bound_box];target=sum(points,Vector())/len(points);records=[]
 for i in range(8):
  angle=i*math.pi/4;direction=Vector((math.sin(angle)*math.cos(math.radians(35)),-math.cos(angle)*math.cos(math.radians(35)),math.sin(math.radians(35))));right=Vector((0,0,1)).cross(direction).normalized();up=direction.cross(right);width=max(p.dot(right)for p in points)-min(p.dot(right)for p in points);height=max(p.dot(up)for p in points)-min(p.dot(up)for p in points);scale=max(width,height*4/3)*1.08
  camera(scene,target,direction,768,576,scale);scene.render.filepath=str(out/f'view-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name);records.append(dict(file=f'view-{i}.png',sha256=sha(out/f'view-{i}.png'),direction=list(direction),target=list(target),scale=scale))
 sheet=Image.new('RGB',(1536,4*604),'#292929');draw=ImageDraw.Draw(sheet)
 for i in range(8):sheet.paste(Image.open(out/f'view-{i}.png'),((i%2)*768,(i//2)*604+28));draw.text(((i%2)*768+5,(i//2)*604+7),'Original game camera' if i==0 else f'Orbit {i}',fill='white')
 sheet.save(out/'actual8.png')
 with bpy.data.libraries.load(str(BANK),link=False)as(src,dst):dst.objects=list(src.objects)
 bank=[o for o in dst.objects if o and o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank'];assert bank
 for o in bank:link(scene,o);o.hide_render=False
 bpy.context.view_layer.update();bank_signatures={o.name:geometry(o)for o in bank};prior=bpy.data.images.load(str(OUT/'restart4-remaining-floor-bake-v1/composite.png'),check_existing=False)
 for label,image in [('approved-before',prior),('candidate-after',candidate_image)]:
  node.image=image
  for view,direction in [('native',RAY),('reverse',Vector((0,.819152,.573576)))]:
   camera(scene,Vector((1370,-30,0)),direction,768,576,150);scene.render.filepath=str(out/f'bank-{view}-{label}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
 node.image=candidate_image
 compare=Image.new('RGB',(1536,1208),'#292929');draw=ImageDraw.Draw(compare)
 for row,view in enumerate(['native','reverse']):
  for col,label in enumerate(['approved-before','candidate-after']):compare.paste(Image.open(out/f'bank-{view}-{label}.png'),(col*768,row*604+28));draw.text((col*768+5,row*604+7),view+' '+label,fill='white')
 compare.save(out/'bank-contact-before-after.png');assert geometry(ground)==sig and bank_signatures=={o.name:geometry(o)for o in bank} and np.array_equal(atlas(ground)[1],pixels)
 write_json(out/'render-evidence.json',dict(status='saved model reviewed renders; visual review pending',model_sha256=sha(D/'model.blend'),atlas_sha256=sha(D/'composite.png'),bank_sha256=sha(BANK),ground_geometry_exact=True,bank_geometry_exact=True,ground_reopened_rgba_exact=True,no_second_model_save=True,first_view='Original game camera',views=records,files={p.name:sha(p)for p in out.glob('*.png')}))
 print('REVIEW COMPLETE',flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
