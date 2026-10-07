"""Inspect saved chain motion against every native frame within a small budget."""
import hashlib, json, math, shutil, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-supported-motion-v1';OUT=BASE/'review'
RESUME='--orbits-only' in sys.argv
if OUT.exists() and not RESUME:raise FileExistsError(OUT)
if RESUME:assert (OUT/'native45-comparison.png').is_file()
def budget(reserve=0):
    used=sum(p.stat().st_size for b in (BASE,WORK/'restart2/winch-supported-hardware-v1',WORK/'restart2/winch-guided-entry-candidate-v1') for p in b.rglob('*') if p.is_file())
    review=sum(p.stat().st_size for p in OUT.rglob('*') if p.is_file())
    assert used+reserve<20*1024**2
    assert shutil.disk_usage(ROOT).free>8*1024**3+20*1024**2-used
budget();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0,str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest,labeled_copy
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;budget();OUT.mkdir(exist_ok=RESUME)
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));cam=scene.objects['Winch0'];cam.location=Vector((2410,-930/s,0))+back*10000;cam.data.ortho_scale=110
montage=Image.new('RGB',(1200,2160),'#303840');draw=ImageDraw.Draw(montage)
for i,f in enumerate([] if RESUME else frames):
    scene.frame_set(i*2);bpy.context.view_layer.update();budget();scene.render.resolution_x=120;scene.render.resolution_y=220
    render_views(scene.name,{'native':cam.name},OUT/f'native/{i:02d}',modes=('textured',),width=120)
    tile=Image.new('RGBA',(60,110),'#303840');tile.alpha_composite(Image.open(source/f['image']).convert('RGBA'),(f['bbox'][0]-2380,f['bbox'][1]-875))
    x,y=(i%5)*240,(i//5)*240;draw.text((x+3,y+3),f'{i:02d} source / saved model',fill='white');montage.paste(tile.resize((120,220),Image.Resampling.NEAREST).convert('RGB'),(x,y+20));im=Image.open(OUT/f'native/{i:02d}/native-textured.png');montage.paste(im,(x+120,y+20),im)
if not RESUME:
    budget();montage.save(OUT/'native45-comparison.png')
own=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004']
for o in scene.objects:
    if o.type=='MESH':o.hide_render=o not in own
for frame in (0,36):
    scene.frame_set(frame*2);bpy.context.view_layer.update();points=[o.matrix_world@v.co for o in own for v in o.data.vertices];center=(Vector(tuple(min(p[i] for p in points) for i in range(3)))+Vector(tuple(max(p[i] for p in points) for i in range(3))))/2;rows=[];folder=OUT/f'pose{frame:02d}';budget();folder.mkdir()
    for i in range(8):
        camera=scene.objects[f'Winch{i}'];camera.location=center+(camera.matrix_world.to_quaternion()@Vector((0,0,1)))*10000;bpy.context.view_layer.update();inv=camera.matrix_world.to_quaternion().inverted();pp=[inv@(p-center) for p in points];camera.data.ortho_scale=max(max(p.y for p in pp)-min(p.y for p in pp),(max(p.x for p in pp)-min(p.x for p in pp))*320/224)*1.25;rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(row) for row in camera.matrix_world],'ortho_scale':camera.data.ortho_scale});budget();scene.render.resolution_x=224;scene.render.resolution_y=320;render_views(scene.name,{f'view-{i}':camera.name},folder/f'renders/view-{i}',modes=('solid',),width=224)
    budget();(folder/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows},indent=2)+'\n');audit_manifest(folder/'views.json');sheet=Image.new('RGBA',(896,640))
    for i in range(8):sheet.paste(Image.open(folder/f'renders/view-{i}/view-{i}-solid.png'),((i%4)*224,(i//4)*320))
    budget();sheet.save(folder/'solid8.png');budget();labeled_copy(folder/'solid8.png',folder/'solid8-native-labeled.png')
budget();(OUT/'review.json').write_text(json.dumps({'model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'native_frames':45,'solid_poses':[0,36],'views_per_pose':8,'limits':'Constant frame hold; finite pose checks; inferred loop drive and collar restraint, not observed gearing; private review only'},indent=2)+'\n');print('BOUNDED SAVED MOTION REVIEW COMPLETE')
