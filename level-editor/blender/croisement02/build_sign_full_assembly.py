"""Bind filled rotating signs and painted shadows to all five native placements."""
import json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'state-sign-candidate';source=base/'phase-appearance-v1/model.blend';evidence=base/'phase-appearance-v1/evidence.json';source_hash=sha(source);assert source_hash==json.loads(evidence.read_text())['model_sha256'];placement_path=base/'placement-audit.json';placement=json.loads(placement_path.read_text());order_path=base/'native-order-reference-v3/manifest.json';order=json.loads(order_path.read_text());dst=base/'five-instances-v3';dst.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;templates=[o for o in scene.objects if o.type in {'MESH','EMPTY'}];assert len([o for o in templates if o.type=='MESH'])==96;rows=[]
 for row in placement['records']:
  assert abs(row['physical_clearance'])<.01
  index=row['target_index'];outer=bpy.data.objects.new(f'Mission S03 FoB MP sign {index}',None);scene.collection.objects.link(outer);outer.location=Vector(row['world_anchor']);outer['source_node']=f'mission-panneau-{index}';outer['native_target_index']=index;outer['mission_visibility']=row['mission'];outer['native_placement_json']=json.dumps(row['native_target'],sort_keys=True);outer['native_implicit_z']=row['native_implicit_z'];copies={}
  for template in templates:
   obj=template.copy();obj.name=f'{outer.name} {template.name}';scene.collection.objects.link(obj);copies[template]=obj;obj['source_node']=outer['source_node'];obj['mission_visibility']=row['mission']
  for template,obj in copies.items():obj.parent=copies.get(template.parent,outer)
  rows.append(dict(target_index=index,root=outer.name,parts=[o.name for o in copies.values()],world_anchor=row['world_anchor'],native_target=row['native_target'],mission=row['mission'],reusable_asset_id='croisement02-mission-rotating-sign'))
 for obj in templates:bpy.data.objects.remove(obj,do_unlink=True)
 for obj in list(scene.objects):
  if obj.type not in {'MESH','EMPTY'}:bpy.data.objects.remove(obj,do_unlink=True)
 scene.frame_set(1);bpy.ops.wm.save_as_mainfile(filepath=str(dst/'model.blend'));digest=sha(dst/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dst/'model.blend'));scene=bpy.context.scene;assert len([o for o in scene.objects if o.type=='MESH'])==480;assert len({o.data for o in scene.objects if o.type=='MESH'})==34
 for phase in range(33):
  scene.frame_set(1+phase*2)
  for row in rows:
   outer=scene.objects[row['root']];assert (outer.location-Vector(row['world_anchor'])).length<1e-4;shadows=[scene.objects[n]for n in row['parts']if 'native_frame'in scene.objects[n]];assert [o['native_frame']for o in shadows if o.scale.x>.5]==[phase%32];bodies=[scene.objects[n]for n in row['parts']if 'native_body_frame'in scene.objects[n]];assert len([o for o in bodies if o.scale.x>.5])==2 and all(o['native_body_frame']==phase%32 for o in bodies if o.scale.x>.5)
 data=bpy.data.cameras.new('Native placed sign review');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=96;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.cycles.transparent_max_bounces=128;sheet=Image.new('RGB',(5*384,4*408),(80,80,80));baseline=Image.open(OUT/'baseline/covered.png').convert('RGBA');animations=json.loads((OUT/'animation-references/manifest.json').read_text())['animations'];frames=next(p for p in json.loads((OUT/'state-target-evidence/manifest.json').read_text())['profiles']if p['id']=='TG_Panel-12')['rows'][0]['frames'];comparisons=[]
 for column,row in enumerate(rows):
  index=row['target_index'];t=row['native_target'];x,y=t['position_x'],t['position_y'];box=(x-48,y-64,x+48,y+32);target=Vector((x,-(y-16)/SIN,0));camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();selected=set(row['parts']);foreground=next(r for r in order['records']if r['target_index']==index)
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o.name not in selected
  for n,phase in enumerate([0,8,16,24]):
   scene.frame_set(1+phase*2);path=dst/f'target-{index}-pose-{phase:02}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);actual=Image.open(path).convert('RGBA');background=baseline.crop(box).resize((384,384),Image.Resampling.NEAREST);native=background.copy();f=frames[phase];sprite=Image.open(f['image']).convert('RGBA').resize((Image.open(f['image']).width*4,Image.open(f['image']).height*4),Image.Resampling.NEAREST);native.alpha_composite(sprite,((x+int(f['offset'][0])-box[0])*4,(y+int(f['offset'][1])-box[1])*4));background.alpha_composite(actual)
   for a in foreground['overlapping_animations']:
    assert a['after_sign'],'Earlier overlay must be composited before both signs';anim=next(r for r in animations if r['index']==a['index']);af=anim['frames'][0];fx,fy,fw,fh=af['bbox'];overlay=Image.open(af['image']).convert('RGBA').resize((fw*4,fh*4),Image.Resampling.NEAREST)
    for canvas in [native,background]:canvas.alpha_composite(overlay,((fx-box[0])*4,(fy-box[1])*4))
   pair=Image.new('RGB',(768,384));pair.paste(native.convert('RGB'),(0,0));pair.paste(background.convert('RGB'),(384,0));pair.save(dst/f'target-{index}-pose-{phase:02}-native-comparison.png');sheet.paste(background.convert('RGB'),(column*384,n*408));ImageDraw.Draw(sheet).text((column*384+4,n*408+386),f'Target {index}, sign pose {phase}, overlay frame 0',fill='white');comparisons.append(str(dst/f'target-{index}-pose-{phase:02}-native-comparison.png'))
 sheet.save(dst/'five-instances-four-poses.png');assert sha(source)==source_hash;assert sha(dst/'model.blend')==digest;write_json(dst/'assembly.json',dict(status='Private complete appearance assembly; independent visual review and scene integration pending',model_sha256=digest,reusable_model_sha256=source_hash,placement_sha256=sha(placement_path),native_order_sha256=sha(order_path),instances=rows,mesh_count=480,shared_mesh_datablocks=34,poses_checked=33,shadow_instances_per_pose=5,timing=dict(ticks_per_second=25,cycle_ticks=64,poses=32,pose_ticks=2),comparisons=comparisons,limitations=['Controlled proof places actual saved sign materials over native static artwork and native frame-zero overlays; this is not a complete physical scene integration render.','Ground-shadow source-role assignment remains explicitly inferred near the post foot.','Independent geometry/appearance review and catalog/export/editor integration remain pending.']))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
