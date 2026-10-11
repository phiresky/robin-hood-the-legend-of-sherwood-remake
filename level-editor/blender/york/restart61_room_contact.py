"""Read-only protected gatehouse contact review for the private room cover."""
import json,hashlib,math,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-contact-v1';assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3
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
base=W/'restart42-channel-assembly-v1/model.blend';cover=W/'restart61-room-geometry-v2/model.blend';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();assert sha(base)=='9e3d3c48c7ce207b2e6c3dcfd9066d4c54fa5d76de729d310a02f19d9451c6aa'
bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.context.scene;protected={o.name:hashlib.sha256(json.dumps([[list(v.co) for v in o.data.vertices],[list(r) for r in o.matrix_world]]).encode()).hexdigest() for o in scene.objects if o.type=='MESH'}
with bpy.data.libraries.load(str(cover),link=False) as (a,b):b.objects=['Room cover solid folded masonry']
o=b.objects[0];scene.collection.objects.link(o);bpy.context.view_layer.update();scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=320;scene.render.resolution_y=384;s,c=math.sin(math.radians(35)),math.cos(math.radians(35));points=[o.matrix_world@v.co for v in o.data.vertices];cameras={}
for i,yaw in enumerate(range(0,360,45)):
 a=math.radians(yaw);back=Vector((math.sin(a)*c,-math.cos(a)*c,s));q=(-back).to_track_quat('-Z','Y');center=sum(points,Vector())/len(points);d=bpy.data.cameras.new(f'Room contact {i}');cam=bpy.data.objects.new(d.name,d);scene.collection.objects.link(cam);d.type='ORTHO';d.clip_end=20000;d.ortho_scale=250;cam.location=center+back*10000;cam.rotation_euler=q.to_euler();cameras[f'view-{i}']=cam.name
OUT.mkdir()
for state,hidden in [('initial',False),('applied',True)]:
 o.hide_render=hidden;render_views(scene.name,cameras,OUT/state,modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
 for i in range(8):sheet.paste(Image.open(OUT/state/f'view-{i}-textured.png'),((i%4)*320,(i//4)*384))
 sheet.save(OUT/state/'eight.png')
for name,digest in protected.items():
 ob=scene.objects[name];assert hashlib.sha256(json.dumps([[list(v.co) for v in ob.data.vertices],[list(r) for r in ob.matrix_world]]).encode()).hexdigest()==digest
(OUT/'report.json').write_text(json.dumps({'status':'CONTACT_REVIEW_PENDING','protected_model_sha256':sha(base),'candidate_sha256':sha(cover),'protected_meshes_exact':len(protected),'notes':'Cover is independent patch005; gate pose remains unchanged. No static scene changes were saved.'},indent=2)+'\n')
