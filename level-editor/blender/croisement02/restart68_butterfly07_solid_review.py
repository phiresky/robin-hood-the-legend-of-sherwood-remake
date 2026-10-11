"""Eight solid views of the exact imported flight model, original camera first."""
import sys,json,math,hashlib
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
BASE=ROOT/'level-editor/work/croisement02-refinement';OUT=BASE/'restart68-butterfly07-solid-review-v1';MODEL=BASE/'restart67-butterfly07-fullpath-export-v1/butterfly07-fullpath.glb'
def main():
 assert not OUT.exists();OUT.mkdir();bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(MODEL));root=bpy.data.objects['Butterfly07 full inferred flight path'];s.frame_set(5)
 mat=bpy.data.materials.new('Solid geometry review');mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.6,.6,.6,1)
 for o in s.objects:
  if o.type=='MESH':
   for i in range(len(o.data.materials)):o.data.materials[i]=mat
 s.render.engine='CYCLES';s.cycles.samples=8;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_x=s.render.resolution_y=256;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None';s.world=bpy.data.worlds.new('Neutral');s.world.use_nodes=True;s.world.node_tree.nodes['Background'].inputs[0].default_value=(.5,.5,.5,1)
 light=bpy.data.lights.new('Soft directional','SUN');light.energy=2;obj=bpy.data.objects.new(light.name,light);s.collection.objects.link(obj);obj.rotation_euler=(.5,-.6,-.4)
 camera=bpy.data.cameras.new('Original art camera first');camera.type='ORTHO';camera.ortho_scale=24;cam=bpy.data.objects.new(camera.name,camera);s.collection.objects.link(cam);s.camera=cam
 for i in range(8):
  angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*math.cos(math.radians(35)),math.sin(angle)*math.cos(math.radians(35)),math.sin(math.radians(35))));center=root.matrix_world.translation;cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();s.render.filepath=str(OUT/f'{i}.png');bpy.ops.render.render(write_still=True)
 from PIL import Image
 sheet=Image.new('RGB',(1024,512),'#252525')
 for i in range(8):im=Image.open(OUT/f'{i}.png').convert('RGBA');sheet.paste(im,(i%4*256,i//4*256),im)
 sheet.save(OUT/'solid-eight.png');(OUT/'report.json').write_text(json.dumps(dict(model_sha256=hashlib.sha256(MODEL.read_bytes()).hexdigest(),phase=2,original_camera_first=True),indent=2)+'\n')
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
