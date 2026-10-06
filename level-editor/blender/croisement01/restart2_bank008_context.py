"""Read-only approved joint context with isolated soil material applied temporarily."""
import json,math,sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';dest=R/'approved-bank008-fill-v1/joint-context-v4';dest.mkdir(exist_ok=False);acquire();records=[];box=[0,225,155,390];left,top,right,bottom=box;width,height=right-left,bottom-top;sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35))
for label,folder in [('Approved joint before soil fill','tree01-soil-joint-v10'),('Same joint with inferred soil texture','tree01-soil-joint-v10')]:
 worker=R/folder/'assets/croisement01-tree-01';bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));cfg=json.loads((worker/'workspace.json').read_text());scene=bpy.data.scenes.new(label);scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256;scene.world=bpy.data.worlds.new('Neutral ambient');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs['Color'].default_value=(.35,.35,.35,1);scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
 materials={}
 bank=None
 if label.startswith('Same'):
  filled=R/'approved-bank008-fill-v1/croisement01-tree01-bank008-joint/baked-v1-luminance/worker.blend'
  with bpy.data.libraries.load(str(filled),link=False) as (src,dst):dst.objects=[n for n in src.objects if n=='Terrain 008 / Source part 008']
  assert len(dst.objects)==1;bank=dst.objects[0]
  chain=bank
  while chain is not None:
   if chain.name not in bpy.context.scene.objects:bpy.context.scene.collection.objects.link(chain)
   chain=chain.parent
  bpy.context.view_layer.update()
 for name,color in [('changed bank008',(.36,.31,.24,1)),('unchanged terrain',(.28,.28,.28,1))]:
  mat=bpy.data.materials.new(name);mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=color;materials[name]=mat
 for obj in list(bpy.data.collections[cfg['collection_name']].all_objects):
  terrain=obj.get('source_node') in {'ground'}|{f'building-{i:03d}' for i in range(10)}
  own=obj.get('asset_group')==cfg['asset_id']
  if obj.type!='MESH' or not (terrain or own):continue
  copied=obj.copy();copied.parent=None;copied.matrix_world=obj.matrix_world.copy();copied.hide_render=False
  if terrain:
   if obj.get('source_node')=='building-008' and bank is not None:
    assert len(obj.data.vertices)==len(bank.data.vertices)
    errors=[(obj.matrix_world@a.co-bank.matrix_world@b.co).length for a,b in zip(obj.data.vertices,bank.data.vertices)];print('BANK TRANSFORM',list(obj.matrix_world),list(bank.matrix_world),max(errors));assert max(errors)<1e-5
    copied.data=bank.data;copied.matrix_world=bank.matrix_world.copy()
   else:
    copied.data=obj.data.copy();copied.data.materials.clear();copied.data.materials.append(materials['changed bank008' if obj.get('source_node')=='building-008' else 'unchanged terrain'])
    for face in copied.data.polygons:face.material_index=0
  scene.collection.objects.link(copied)
 light=bpy.data.lights.new('Native joint light','SUN');light.energy=1.5;sun=bpy.data.objects.new(light.name,light);scene.collection.objects.link(sun);sun.rotation_euler=Vector((-.6,-.4,-.7)).to_track_quat('-Z','Y').to_euler()
 target=Vector(((left+right)/2,-(top+bottom)/2/sine,0));camera_data=bpy.data.cameras.new('Native game camera');camera_data.type='ORTHO';camera_data.sensor_fit='HORIZONTAL';camera_data.ortho_scale=width;camera_data.clip_end=20000;camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera);camera.location=target+Vector((0,-cosine,sine))*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
 scene.render.resolution_x=width;scene.render.resolution_y=height;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';path=dest/('filled.png' if bank is not None else 'approved-geometry.png');scene.render.filepath=str(path);bpy.ops.render.render(write_still=True,scene=scene.name);records.append(dict(label=label,worker_sha256=sha(worker/'model.blend'),image_sha256=sha(path),image=path.name))
source=Image.open(ROOT/'level-editor/work/croisement01-refinement/baseline/covered.png').convert('RGBA').crop(box);source.save(dest/'native-source.png');panels=[('Original native artwork',source)]+[(row['label'],Image.open(dest/row['image']).convert('RGBA')) for row in records];board=Image.new('RGB',(width*4*3,height*4+45),'#444444');draw=ImageDraw.Draw(board)
for i,(label,im) in enumerate(panels):
 draw.text((i*width*4+5,5),label+' - native camera',fill='white');enlarged=im.resize((width*4,height*4),Image.Resampling.NEAREST);board.paste(enlarged,(i*width*4,35),enlarged)
board.save(dest/'comparison.png');(dest/'evidence.json').write_text(json.dumps(dict(status='Soil appearance comparison on unchanged approved joint; neighboring gray terrain remains context',source_crop=box,views=records,comparison_sha256=sha(dest/'comparison.png'),legend='Left joint retains diagnostic tan bank. Right applies candidate soil to exact bank008 world geometry. Gray neighboring walls remain unchanged context; native wood untouched. No whole terrain completion claim.',source_ownership= str(R/'tree01-soil-joint-v1/soil-source-ownership.json')),indent=2)+'\n')
