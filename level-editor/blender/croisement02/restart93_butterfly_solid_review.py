"""Render matching solid views of a pinned private butterfly model at native tick zero."""
import argparse,sys,json,hashlib,shutil
from pathlib import Path
import bpy
from mathutils import Matrix
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--model',type=Path,required=True);p.add_argument('--review',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);memory={k:int(v.split()[0])*1024 for k,v in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())};assert memory['MemAvailable']>=6*1024**3 and shutil.disk_usage(ROOT).free>=10*1024**3;a.output.mkdir(exist_ok=False);review=json.loads(a.review.read_text());assert sha(a.model)==review['model_sha256']
 bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(a.model));s.frame_set(0)
 mat=bpy.data.materials.new('Solid geometry review');mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.6,.6,.6,1)
 for o in s.objects:
  if o.type=='MESH':
   for i in range(len(o.data.materials)):o.data.materials[i]=mat
 s.render.engine='CYCLES';s.cycles.samples=8;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_x=s.render.resolution_y=256;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None';s.world=bpy.data.worlds.new('Neutral');s.world.use_nodes=True;s.world.node_tree.nodes['Background'].inputs[0].default_value=(.5,.5,.5,1)
 light=bpy.data.lights.new('Soft directional','SUN');light.energy=2;o=bpy.data.objects.new(light.name,light);s.collection.objects.link(o);o.rotation_euler=(.5,-.6,-.4)
 camdata=bpy.data.cameras.new('Original art camera first');camdata.type='ORTHO';cam=bpy.data.objects.new(camdata.name,camdata);s.collection.objects.link(cam);s.camera=cam
 for i,v in enumerate(review['views']):
  cam.matrix_world=Matrix(v['camera_matrix_world']);camdata.ortho_scale=v['ortho_scale'];s.render.filepath=str(a.output/f'{i}.png');bpy.ops.render.render(write_still=True)
 sheet=Image.new('RGB',(1024,512),'#252525')
 for i in range(8):im=Image.open(a.output/f'{i}.png').convert('RGBA');sheet.paste(im,(i%4*256,i//4*256),im)
 sheet.save(a.output/'solid-eight.png');(a.output/'report.json').write_text(json.dumps(dict(model_sha256=sha(a.model),textured_review_sha256=sha(a.review),tick=0,original_camera_first=True,scope='Matching saved solid cameras only; not approval.'),indent=2)+'\n')
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
