"""Review a reopened empty endpoint using eight native-first camera directions."""
import sys,json,math
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS
BASE=OUT/'restart2-state/net-empty01-v9'
def main():
 dest=BASE/'review';dest.mkdir(exist_ok=False);expected=json.loads((BASE/'report.json').read_text())['model_sha256']
 if sha(BASE/'model.blend')!=expected:raise ValueError('Endpoint changed')
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];lo=Vector(tuple(min(p[i]for p in points)for i in range(3)));hi=Vector(tuple(max(p[i]for p in points)for i in range(3)));center=(lo+hi)/2;scene.camera.data.ortho_scale=(hi-lo).length*1.2;records=[]
  for i in range(8):
   a=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler()
   for mode in ['actual','solid']:
    scene.view_layers[0].material_override=bpy.data.materials['Unobserved net surfaces'] if mode=='solid' else None;file=dest/f'{i:02}-{mode}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);records.append({'view':i,'mode':mode,'image':file.name,'direction':list(direction),'sha256':sha(file)})
  for mode in ['actual','solid']:
   sheet=Image.new('RGB',(1024,512),'#333333')
   for i in range(8):
    image=Image.open(dest/f'{i:02}-{mode}.png').convert('RGBA').resize((256,256));background=Image.new('RGBA',image.size,'#333333');background.alpha_composite(image);sheet.paste(background.convert('RGB'),((i%4)*256,(i//4)*256))
   sheet.save(dest/f'{mode}-eight.png')
  write_json(dest/'manifest.json',{'model_sha256':expected,'records':records,'native_first':True})
 finally:release()
if __name__=='__main__':main()
