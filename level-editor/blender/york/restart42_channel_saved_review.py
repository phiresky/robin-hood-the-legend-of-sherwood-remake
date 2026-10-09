"""Render unclipped saved gatehouse context and explicitly cropped guide details."""
import json,hashlib,math,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';MODEL=W/'restart42-channel-assembly-v1/model.blend';OUT=MODEL.parent/'complete-review-v1';assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire(slots=2)
import bpy
from mathutils import Vector
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();digest=sha(MODEL);validation=json.loads((MODEL.parent/'validation.json').read_text());assert digest==validation['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(MODEL));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;gate=bpy.data.objects['scenery-york-castle-portcullis'];matrix=gate.matrix_world.copy();motion=json.loads((W/'jamb-clearance-candidate-v1/motion-proposal.json').read_text());lift=motion['rows'][44]['nominal_lift_world_z'];points=[]
for o in scene.objects:
 if o.type=='MESH':
  points.extend(o.matrix_world@v.co for v in o.data.vertices)
  if o==gate:points.extend(o.matrix_world@v.co+Vector((0,0,lift))for v in o.data.vertices)
def camera(name,back,center,scale):
 data=bpy.data.cameras.new(name);cam=bpy.data.objects.new(name,data);scene.collection.objects.link(cam);data.type='ORTHO';data.ortho_scale=scale;data.clip_end=20000;cam.location=center+back*10000;cam.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();return cam
cameras={};framing=[]
for i,yaw in enumerate(range(0,360,45)):
 a,b=math.radians(yaw),math.radians(35);back=Vector((math.sin(a)*math.cos(b),-math.cos(a)*math.cos(b),math.sin(b)));rotation=(-back).to_track_quat('-Z','Y');right=rotation@Vector((1,0,0));up=rotation@Vector((0,1,0));rx=[p.dot(right)for p in points];uy=[p.dot(up)for p in points];dz=[p.dot(back)for p in points];center=right*((min(rx)+max(rx))/2)+up*((min(uy)+max(uy))/2)+back*((min(dz)+max(dz))/2);scale=max(max(uy)-min(uy),(max(rx)-min(rx))*384/320)*1.12;cam=camera('Complete native'if i==0 else'Complete '+str(i),back,center,scale);cameras[f'view-{i}']=cam.name;framing.append({'view':i,'yaw':yaw,'elevation':35,'ortho_scale':scale,'all_vertices_in_frame':True})
scene.render.resolution_x=320;scene.render.resolution_y=384
for state,offset in [('initial',0),('applied',lift)]:
 gate.matrix_world=matrix.copy();gate.matrix_world.translation.z+=offset;bpy.context.view_layer.update();dest=OUT/state;render_views(scene.name,cameras,dest,modes=('textured',),width=320)
 for mode in ['textured']:
  views=[Image.open(dest/f'view-{i}-{mode}.png').convert('RGBA')for i in range(8)];sheet=Image.new('RGBA',(1280,768))
  for i,im in enumerate(views):sheet.paste(im,((i%4)*320,(i//4)*384))
  sheet.save(dest/f'{mode}-eight.png')
plan=json.loads((W/'approved-shed-jamb-motion-integration-v1/gatehouse-channel-proposal.json').read_text());origin=Vector(plan['channel_basis']['origin']);axis=Vector(plan['channel_basis']['horizontal_axis']);normal=Vector(plan['channel_basis']['normal']);gate.matrix_world=matrix.copy()
# Detail views are deliberate cutaways: hide other parts and the approved jamb.
for o in scene.objects:
 if o.type=='MESH':o.hide_render=o.get('source_node')not in ['building-778','building-779']or o.name=='building-778-portcullis-jamb-return'
names={}
for label,u,sign in [('left',0,1),('right',82,-1)]:
 for side in [-1,1]:
  back=(axis*sign+normal*side*.65+Vector((0,0,.15))).normalized();center=origin+axis*u+Vector((0,0,182-origin.z));cam=camera(f'{label}-{side}',back,center,175);names[f'{label}-{side}']=cam.name
scene.render.resolution_x=200;scene.render.resolution_y=320;render_views(scene.name,names,OUT/'guide-details',modes=('textured',),width=200)
for mode in ['textured']:
 views=[Image.open(OUT/f'guide-details/{name}-{mode}.png').convert('RGBA')for name in names];sheet=Image.new('RGBA',(800,320))
 for i,im in enumerate(views):sheet.paste(im,(i*200,0))
 sheet.save(OUT/f'guide-details/{mode}-four.png')
assert sha(MODEL)==digest;report={'status':'READ_ONLY_COMPLETE_AND_DETAIL_REVIEW','model_sha256':digest,'views':framing,'guide_detail_scope':'Deliberately cropped cutaways of778/779only; other gatehouse parts and approved jamb hidden to expose new interior walls.','external_images':json.loads((W/'gatehouse-channel-candidate-v3/validation.json').read_text())['external_image_hashes']};(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in MODEL.parent.rglob('*')if p.is_file())<32*1024**2;print(json.dumps({'out':str(OUT),'model_sha256':digest}))
