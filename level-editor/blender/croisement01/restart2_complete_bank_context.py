"""Compare baseline and proposed local soil from one unchanged native camera."""
import argparse,json,math,sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
parser=argparse.ArgumentParser();parser.add_argument('--tree',type=int,required=True);parser.add_argument('--parent',required=True);parser.add_argument('--candidate',required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);n=args.tree;asset=f'croisement01-tree-{n:02d}';changed={4:{'building-003','building-004'},5:{'building-006','building-007'}}[n]
R=ROOT/'level-editor/work/croisement01-refinement/restart2';changed=set(json.loads((R/args.candidate/'construction.json').read_text())['changed_nodes']);dest=R/args.candidate/'bank-native-context-v1';dest.mkdir(exist_ok=False);acquire();records=[];box={4:[40,460,355,730],5:[210,105,485,350]}[n];left,top,right,bottom=box;width,height=right-left,bottom-top;sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
for label,folder in [('Parent archived banks',args.parent),('Complete bank proposal',args.candidate)]:
 worker=R/folder/'assets'/asset;bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));cfg=json.loads((worker/'workspace.json').read_text());scene=bpy.data.scenes.new(label);scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256;scene.world=bpy.data.worlds.new('Neutral ambient');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs['Color'].default_value=(.35,.35,.35,1);scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
 materials={}
 for name,color in [('proposed bank owners',(.36,.31,.24,1)),('unchanged terrain',(.28,.28,.28,1))]:
  mat=bpy.data.materials.new(name);mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=color;materials[name]=mat
 for obj in list(bpy.data.collections[cfg['collection_name']].all_objects):
  terrain=obj.get('source_node') in {'ground','tree05-local-soil-joint'}|{f'building-{i:03d}' for i in range(10)}
  own=obj.get('asset_group')==cfg['asset_id']
  if obj.type!='MESH' or not (terrain or own):continue
  copied=obj.copy();copied.parent=None;copied.matrix_world=obj.matrix_world.copy();copied.hide_render=False
  if terrain:
   copied.data=obj.data.copy();copied.data.materials.clear();copied.data.materials.append(materials['proposed bank owners' if obj.get('source_node') in changed else 'unchanged terrain'])
   for face in copied.data.polygons:face.material_index=0
  scene.collection.objects.link(copied)
 light=bpy.data.lights.new('Native joint light','SUN');light.energy=1.5;sun=bpy.data.objects.new(light.name,light);scene.collection.objects.link(sun);sun.rotation_euler=Vector((-.6,-.4,-.7)).to_track_quat('-Z','Y').to_euler()
 target=Vector(((left+right)/2,-(top+bottom)/2/sine,0));camera_data=bpy.data.cameras.new('Native game camera');camera_data.type='ORTHO';camera_data.sensor_fit='HORIZONTAL';camera_data.ortho_scale=width;camera_data.clip_end=20000;camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera);camera.location=target+Vector((0,-cosine,sine))*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
 scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';path=dest/(folder+'.png');scene.render.filepath=str(path);bpy.ops.render.render(write_still=True,scene=scene.name);records.append(dict(label=label,worker_sha256=sha(worker/'model.blend'),image_sha256=sha(path),image=path.name))
source=Image.open(ROOT/'level-editor/work/croisement01-refinement/baseline/covered.png').convert('RGBA').crop(box);source.save(dest/'native-source.png');panels=[('Original native artwork',source)]+[(row['label'],Image.open(dest/row['image']).convert('RGBA')) for row in records];board=Image.new('RGB',(width*4*3,height*4+45),'#444444');draw=ImageDraw.Draw(board)
for i,(label,im) in enumerate(panels):
 draw.text((i*width*4+5,5),label+' - native camera',fill='white');enlarged=im.resize((width*4,height*4),Image.Resampling.NEAREST);board.paste(enlarged,(i*width*4,35),enlarged)
board.save(dest/'comparison.png');(dest/'evidence.json').write_text(json.dumps(dict(status='Geometry comparison only; terrain colors are diagnostic overrides, not approved texture',source_crop=box,views=records,comparison_sha256=sha(dest/'comparison.png'),legend='Tan surfaces are the complete proposed bank pair. Gray neighboring walls remain unchanged coarse context. Wood keeps saved source materials. Off-map and unobserved soil depth is inferred.',source_ownership='Terrain texture ownership not yet prepared; this is geometry diagnostic only.'),indent=2)+'\n')
