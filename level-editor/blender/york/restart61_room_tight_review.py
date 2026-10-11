"""Pixel-aligned source/initial/applied room comparison, independent of winch."""
import json,math,sys,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-tight-v13';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
model=W/'restart61-room-interior-v13/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=252;scene.render.resolution_y=450;s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));d=bpy.data.cameras.new('Exact source crop');cam=bpy.data.objects.new(d.name,d);scene.collection.objects.link(cam);d.type='ORTHO';d.clip_end=20000;d.ortho_scale=150;cam.location=Vector((2396,-940/s,0))+back*10000;cam.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();covers=[o for o in scene.objects if o.get('state_recipe')=='initial_only'];OUT.mkdir()
for state,hidden in [('initial',False),('applied',True)]:
 for o in covers:o.hide_render=hidden
 render_views(scene.name,{'native':cam.name},OUT/state,modes=('textured','solid'),width=252)
crop=(2354,865,2438,1015);base=Image.open(W.parent/'source-states-complete/revealed.png').convert('RGBA');initial=base.copy();patch=Image.open(W.parent/'geometry-pass-01/native-state-source-v1/patch-005/row-0/frame-000.png');initial.alpha_composite(patch,(2356,876));sheet=Image.new('RGB',(1008,482),(36,41,47));draw=ImageDraw.Draw(sheet)
for i,(title,im)in enumerate([('Source: covered',initial.crop(crop).resize((252,450),Image.Resampling.NEAREST)),('Model: covered',Image.open(OUT/'initial/native-textured.png')),('Source: revealed',base.crop(crop).resize((252,450),Image.Resampling.NEAREST)),('Model: revealed',Image.open(OUT/'applied/native-textured.png'))]):
 sheet.paste(im,(i*252,32),im if im.mode=='RGBA' else None);draw.text((i*252+4,8),title,fill='white')
sheet.save(OUT/'comparison.png');(OUT/'report.json').write_text(json.dumps({'model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'crop':crop,'camera_elevation':35,'independent_state':'Patch005 only; winch not overlaid. Source RGB unchanged.','layout':'covered-source,covered-model,revealed-source,revealed-model'},indent=2)+'\n')
