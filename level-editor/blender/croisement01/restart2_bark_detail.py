"""Read-only neutral eight-view bark close-ups of a saved texture candidate."""
import argparse,json,sys,hashlib,math
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));from render_slots import acquire
p=argparse.ArgumentParser();p.add_argument('workspace',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.workspace.resolve();out=w/'inspection/bark-detail8';out.mkdir(exist_ok=False);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();before=sha(w/'model.blend');acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());packet=json.loads((w/'modified/views.json').read_text());scene=bpy.data.scenes.new('Saved bark detail');scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.render.resolution_x=192;scene.render.resolution_y=1536;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.film_transparent=False;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.world=bpy.data.worlds.new('Neutral bark background');scene.world.color=(.12,.12,.12)
objects=[o for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id'] and not o.get('source_node','').startswith('foliage-')];points=[]
for obj in objects:
 copy=obj.copy();copy.parent=None;copy.matrix_world=obj.matrix_world.copy();copy.hide_render=False;scene.collection.objects.link(copy);points.extend(obj.matrix_world@v.co for v in obj.data.vertices)
center=Vector([(min(p[i] for p in points)+max(p[i] for p in points))/2 for i in range(3)]);images=[]
for view in packet['views']:
 matrix=Matrix(view['camera_matrix_world']);direction=matrix.to_3x3()@Vector((0,0,1));data=bpy.data.cameras.new('Bark camera');data.type='ORTHO';data.ortho_scale=100;data.clip_end=20000;data.sensor_fit='HORIZONTAL';camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=center+direction*5000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();scene.camera=camera;scene.render.filepath=str(out/f"view-{view['index']}.png");bpy.ops.render.render(write_still=True,scene=scene.name);im=Image.open(scene.render.filepath).convert('RGB');ImageDraw.Draw(im).text((4,4),'Native game camera' if view['index']==0 else 'View '+str(view['index']),fill='white',stroke_width=1,stroke_fill='black');images.append(im)
sheet=Image.new('RGB',(192*4,1536*2));
for i,im in enumerate(images):sheet.paste(im,((i%4)*192,(i//4)*1536))
sheet.save(out/'sheet.png');assert sha(w/'model.blend')==before;(out/'evidence.json').write_text(json.dumps(dict(model_sha256=before,objects=[o.name for o in objects],sheet_sha256=sha(out/'sheet.png'),native_first=True,scope='Close-up bark and branches only; crown deliberately hidden in temporary render scene, model unchanged',ortho_scale=100,resolution=[192,1536]),indent=2)+'\n')
