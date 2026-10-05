"""Neutral new-volume and bank-contact views; appearance overrides are never saved."""
import sys,json,math
from pathlib import Path
import bpy
from mathutils import Vector
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


def neutral(name,color):
 mat=bpy.data.materials.new(name);mat.use_nodes=True
 node=mat.node_tree.nodes.get('Principled BSDF');node.inputs['Base Color'].default_value=(*color,1);node.inputs['Roughness'].default_value=.9
 return mat


def main(variant='collar-v8'):
 base=OUT/'restart3-tree06-root'/variant;dest=base/'neutral-solid';dest.mkdir(exist_ok=False)
 research=variant.startswith('research-');model=base/('root.blend'if research else 'model.blend');digest=sha(model)
 bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
 bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
 names=[o.name for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']
 expected={n:dict(matrix_world=[list(r)for r in bpy.data.objects[n].matrix_world])for n in names}
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();scene=bpy.context.scene
 root=next(o for o in scene.objects if o.type=='MESH'and (o.get('research_only')if research else o.name=='Northwest Tree 06 / Root collar continuation'))
 original_model=None
 if research:
  root_name=root.name;root_matrix={root_name:dict(matrix_world=[list(r)for r in root.matrix_world])}
  original_model=Path(json.loads((OUT/'restart3-tree06-root/probe.json').read_text())['model'])
  bpy.ops.wm.open_mainfile(filepath=str(original_model));bpy.context.view_layer.update();scene=bpy.context.scene
  roots,root_receipt=append_verified(scene,model,[root_name],root_matrix);root=roots[0]
 original=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-06' and 'wood 'in o.name]
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o!=root and o not in original
 banks,receipt=append_verified(scene,bank,names,expected)
 materials=[neutral('New root: solid blue',(.34,.53,.7)),neutral('Existing wood: solid gray',(.42,.42,.42)),neutral('Bank: solid muted green',(.29,.34,.26))]
 for objects,mat in [([root],materials[0]),(original,materials[1]),(banks,materials[2])]:
  for o in objects:
   o.data.materials.clear();o.data.materials.append(mat)
   for face in o.data.polygons:face.material_index=0
 for o in list(scene.objects):
  if o.type=='LIGHT':bpy.data.objects.remove(o,do_unlink=True)
 world=bpy.data.worlds.new('Neutral root inspection world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Color'].default_value=(.6,.6,.6,1);world.node_tree.nodes['Background'].inputs['Strength'].default_value=.65;scene.world=world
 light=bpy.data.lights.new('Neutral root inspection sun','SUN');light.energy=2;light.angle=.15
 sun=bpy.data.objects.new(light.name,light);scene.collection.objects.link(sun);sun.rotation_euler=Vector((-.5,.5,-1)).to_track_quat('-Z','Y').to_euler()
 camera=configure(scene);scene.cycles.samples=16;camera.data.ortho_scale=145
 center=Vector((650,(-535-COS*47)/SIN,47))
 for mode in ['new-volume','contact']:
  for o in original+banks:o.hide_render=mode=='new-volume'
  sheet=Image.new('RGB',(1536,816),(65,65,65))
  for i in range(8):
   angle=i*math.pi/4;camera_to(camera,center,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)))
   pic=render(scene,dest/f'{mode}-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'))
   ImageDraw.Draw(sheet).text((i%4*384+5,i//4*408+388),f'{mode} {i}: blue new / gray old / green bank',fill='white')
  sheet.save(dest/f'{mode}-eight.png')
 assert sha(model)==digest
 write_json(dest/'report.json',dict(model_sha256=digest,original_model=str(original_model)if original_model else None,original_model_sha256=sha(original_model)if original_model else None,bank_sha256=sha(bank),verified_context=receipt,appearance_override_only=True,no_model_saved=True,native_camera_first=True,legend={'blue':'new root volume','gray':'unchanged approved wood','green':'exact bank context'},scope='Geometry inspection only; original saved-material views remain the appearance authority'))


if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1]if '--'in sys.argv else 'collar-v8')
 finally:release()
