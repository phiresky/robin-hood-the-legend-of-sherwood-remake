"""Original-camera-first close eight views of the bounded initial fence correction."""
import sys,math,json
from pathlib import Path
import bpy
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from review_bank_candidate import camera
from tree_geometry import SIN,COS,RAY

def main():
 variant=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'geometry-v5';d=OUT/'restart3-initial-fence'/variant;out=d/'close8-v1';cut=json.loads((d/'cut-plane-guard.json').read_text());assert cut['status']=='PASS';out.mkdir(exist_ok=False);model=d/'model.blend';assert sha(model)==cut['model_sha256']
 bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-south-field-wattle-fence'];scene=bpy.data.scenes.new('Initial fence changed section close views');bpy.context.window.scene=scene
 for obj in objects:
  scene.collection.objects.link(obj);obj.hide_render=False;parent=obj.parent
  while parent:
   if parent.name not in scene.objects:scene.collection.objects.link(parent)
   parent=parent.parent
 bpy.context.view_layer.update();target=Vector((1094,-1559,35));world=bpy.data.worlds.new('Close review world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.7;scene.world=world
 light=bpy.data.lights.new('Close review sun','SUN');light.energy=2;sun=bpy.data.objects.new('Close review sun',light);scene.collection.objects.link(sun);sun.rotation_euler=(.5,-.6,-.4)
 solid=bpy.data.materials.new('Close solid');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.45,.45,.45,1)
 for mode in ['actual','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid' else None;sheet=Image.new('RGB',(2048,768),'#303030')
  for i in range(8):
   angle=i*math.pi/4;direction=RAY if i==0 else Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));camera(scene,target,direction,512,384,220);scene.render.filepath=str(out/f'{mode}-{i}.png');bpy.ops.render.render(write_still=True,scene=scene.name)
   im=Image.open(scene.render.filepath).convert('RGBA');bg=Image.new('RGBA',im.size,'#303030');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),(i%4*512,i//4*384))
  sheet.save(out/(mode+'8.png'))
 assert sha(model)==cut['model_sha256'];write_json(out/'receipt.json',dict(status='PASS fixed close cameras; no geometry or save',model_sha256=sha(model),cut_guard_sha256=sha(d/'cut-plane-guard.json'),source_camera_first=True,scale=220,target=list(target),scope='Close crop centered on changed initial section; complete unchanged fence retained outside camera frame.',no_model_saved=True))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
