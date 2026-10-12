"""Source-camera scene proof retaining all nearby static geometry and alpha."""
import sys,json,math,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from bpy_extras.object_utils import world_to_camera_view
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[2]/'refinement')]
import qualify as q
from render_slots import acquire,release
S,C=math.sin(math.radians(35)),math.cos(math.radians(35));RAY=Vector((0,-C,S));DEST=q.OUT/'joined02-v1'
def main():
 assert not DEST.exists();assert shutil.disk_usage(q.ROOT).free>10*1024**3+32*1024**2;DEST.mkdir();qualification=json.loads((q.OUT/'qualification.json').read_text());row=qualification['rows'][1];model=q.OUT/'geometry-v2-rounded02/profile-02/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));screen=[o for o in bpy.context.scene.objects if o.type=='MESH'];receivers=[]
 for candidate in row['bounds_candidates']:
  path=q.LIB/candidate['model'];assert q.pin(path)['sha256']==candidate['model_sha256'];before=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(path));added=set(bpy.data.objects)-before;roots=[o for o in added if o.parent not in added];t=candidate['placement']['transform'];assert t['rot_deg']==0
  for root in roots:root.location+=Vector((t['dx'],-t['dy']/S,t['dz']))
  for obj in added:
   if obj.get('gameplay_only'):obj.hide_render=True
  bpy.context.view_layer.update()
  points=np.array([obj.matrix_world@v.co for obj in added if obj.type=='MESH' for v in obj.data.vertices]);uv=np.c_[points[:,0],-S*points[:,1]-C*points[:,2]];box=candidate['projected_aabb'];assert np.all(uv>=np.array(box[:2])-.02) and np.all(uv<=np.array(box[2:])+.02),candidate['id']
  receivers.extend(o for o in added if o.type=='MESH')
 bpy.context.view_layer.update();crop=[row['source_union_box'][0]-35,-35,row['source_union_box'][2]+35,row['source_union_box'][3]+35];w,h=crop[2]-crop[0],crop[3]-crop[1];cx,cy=(crop[0]+crop[2])/2,(crop[1]+crop[3])/2;center=Vector((cx,-cy/S,0));scene=bpy.context.scene;cam=bpy.data.objects.new('Exact source camera',bpy.data.cameras.new('Exact source camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.ortho_scale=max(w,h);cam.data.clip_end=10000;cam.location=center+RAY*4000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=w;scene.render.resolution_y=h;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';scene.render.image_settings.file_format='PNG';scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('Neutral context');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[1].default_value=1
 bpy.context.view_layer.update()
 for sx,sy,ex,ey in [(crop[0],crop[1],0,1),(crop[2],crop[3],1,0)]:
  uv=world_to_camera_view(scene,cam,Vector((sx,-sy/S,0)));assert abs(uv.x-ex)<.0001 and abs(uv.y-ey)<.0001,(uv,ex,ey)
 bg=Image.open(q.ROOT/qualification['background']['path']).convert('RGBA').crop(crop);sheet=Image.new('RGB',(w*3,(h+25)*2),(25,25,25));draw=ImageDraw.Draw(sheet);pins=[]
 for j,phase in enumerate(['initial','applied']):
  for obj in screen:obj.hide_render=obj['source_phase']!=phase;obj.hide_viewport=obj.hide_render
  p=DEST/f'{phase}-joined.png';scene.render.filepath=str(p);bpy.ops.render.render(write_still=True);actual=Image.alpha_composite(bg,Image.open(p).convert('RGBA'));f=row['art']['states']['initial']['frames'][0]if phase=='initial'else row['art']['states']['transition']['frames'][-1];native=bg.copy();native.alpha_composite(q.rgba(f),(int(f['top_left'][0])-crop[0],int(f['top_left'][1])-crop[1]))
  for i,(label,im)in enumerate([('Base artwork',bg),('Native overlay',native),('Static geometry + screen',actual)]):sheet.paste(im.convert('RGB'),(i*w,j*(h+25)+25));draw.text((i*w+3,j*(h+25)+4),phase+' / '+label,fill='white')
  pins.append(q.pin(p))
 sheet.save(DEST/'source-context.png');(DEST/'report.json').write_text(json.dumps(dict(status='PRIVATE_JOINED_SOURCE_CAMERA_REVIEW',screen_model=q.pin(model),source_qualification=q.pin(q.OUT/'qualification.json'),retained_receivers=row['bounds_candidates'],rendered=pins,crop=crop,limits=['Native background is a backplate; nearby published geometry contributes actual depth and alpha above it.','Lighting and mip filtering differ from runtime; this diagnoses geometry/occlusion, not exact final color.','No model/placement modified or published.']),indent=2)+'\n')
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
