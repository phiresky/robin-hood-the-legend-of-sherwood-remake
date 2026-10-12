"""Read-only protected gatehouse contact review for the private room cover."""
import json,hashlib,math,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-interior-contact-v13';REPAIR='--report-only' in sys.argv;assert REPAIR or not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
from mathutils import Vector
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
base=W/'restart61-room-interior-v13/model.blend';cover=base;sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
winch=W/'restart59-approved-winch-assembly-v1/model.blend';assert sha(winch)=='dd3177aa0639bebadcf99a954f92a3908db2d69a681e2ae48b5592beddbed33f'
bpy.ops.wm.open_mainfile(filepath=str(winch));bpy.context.scene.frame_set(0);bpy.context.view_layer.update();poses={o.name:o.matrix_world.copy()for o in bpy.context.scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004'}
bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.context.scene
with bpy.data.libraries.load(str(winch),link=False)as(a,b):b.objects=list(poses)
for ob in b.objects:scene.collection.objects.link(ob)
bpy.context.view_layer.update()
for ob in b.objects:ob.parent=None;ob.animation_data_clear();ob.matrix_world=poses[ob.name]
bpy.context.view_layer.update();protected={o.name:hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(r) for r in o.matrix_world]]).encode()).hexdigest() for o in scene.objects if o.type=='MESH'}
covers=[x for x in scene.objects if x.get('state_recipe')=='initial_only'];o=covers[0];bpy.context.view_layer.update();scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=320;scene.render.resolution_y=384;s,c=math.sin(math.radians(35)),math.cos(math.radians(35));points=[ob.matrix_world@v.co for ob in scene.objects if ob.type=='MESH' for v in ob.data.vertices];cameras={}
for i,yaw in enumerate(range(0,360,45)):
 a=math.radians(yaw);back=Vector((math.sin(a)*c,-math.cos(a)*c,s));q=(-back).to_track_quat('-Z','Y');right=q@Vector((1,0,0));up=q@Vector((0,1,0));rx=[p.dot(right)for p in points];uy=[p.dot(up)for p in points];dz=[p.dot(back)for p in points];center=right*((min(rx)+max(rx))/2)+up*((min(uy)+max(uy))/2)+back*((min(dz)+max(dz))/2);scale=max(max(uy)-min(uy),(max(rx)-min(rx))*384/320)*1.08;d=bpy.data.cameras.new(f'Room contact {i}');cam=bpy.data.objects.new(d.name,d);scene.collection.objects.link(cam);d.type='ORTHO';d.clip_end=20000;d.ortho_scale=scale;cam.location=center+back*10000;cam.rotation_euler=q.to_euler();cameras[f'view-{i}']=cam.name
OUT.mkdir(exist_ok=REPAIR)
if not REPAIR:
 for state,hidden in [('initial',False),('applied',True)]:
 
  for cover in covers:cover.hide_render=hidden
  render_views(scene.name,cameras,OUT/state,modes=('textured','solid'),width=320);sheet=Image.new('RGBA',(1280,768))
  for i in range(8):sheet.paste(Image.open(OUT/state/f'view-{i}-textured.png'),((i%4)*320,(i//4)*384))
  sheet.save(OUT/state/'eight.png')
  sheet=Image.new('RGBA',(1280,768))
  for i in range(8):sheet.paste(Image.open(OUT/state/f'view-{i}-solid.png'),((i%4)*320,(i//4)*384))
  sheet.save(OUT/state/'solid-eight.png')
for name,digest in protected.items():
 ob=scene.objects[name];assert hashlib.sha256(json.dumps([[list(v.co) for v in ob.data.vertices],[list(r) for r in ob.matrix_world]]).encode()).hexdigest()==digest
(OUT/'report.json').write_text(json.dumps({'status':'CONTACT_REVIEW_PENDING','protected_model_sha256':sha(base),'candidate_sha256':sha(base),'protected_meshes_exact':len(protected),'winch_context_sha256':sha(winch),'winch_context_pose':0,'review_modes':['solid','textured'],'unclipped_views':True,'notes':'Cover is independent patch005; gate pose remains unchanged. No static scene changes were saved. Report-only repair may bind completed images after output variable correction.'},indent=2)+'\n')
