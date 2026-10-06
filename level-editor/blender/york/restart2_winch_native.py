"""Render the exact winch candidate at the native source coordinates."""
import json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-geometry-v3';SRC=WORK/'geometry-pass-01/native-state-source-v1'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
record=next(r for r in json.loads((SRC/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
for state,index in [('transition-00',0),('transition-44',44)]:
 out=BASE/state/'native-v2';out.mkdir();bpy.ops.wm.open_mainfile(filepath=str(BASE/state/'model.blend'));scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004' or ('Travelling round part' in o.name and index==0)
 d=bpy.data.cameras.new('Native mechanism');o=bpy.data.objects.new(d.name,d);scene.collection.objects.link(o);d.type='ORTHO';d.ortho_scale=110;d.clip_end=20000;back=Vector((0,-c,s));o.location=Vector((2410,-930/s,0))+back*10000;o.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=360;scene.render.resolution_y=660
 render_views(scene.name,{'native':o.name},out,modes=('textured',),width=360)
 f=frames[index];native=Image.new('RGBA',(60,110),'#303840');native.alpha_composite(Image.open(SRC/f['image']).convert('RGBA'),(f['bbox'][0]-2380,f['bbox'][1]-875));native=native.resize((360,660),Image.Resampling.NEAREST)
 candidate=Image.open(out/'native-textured.png');sheet=Image.new('RGB',(720,685),'#303840');sheet.paste(native,(0,25));sheet.paste(candidate,(360,25),candidate);draw=ImageDraw.Draw(sheet);draw.text((5,7),'Native source',fill='white');draw.text((365,7),'Private geometry hypothesis',fill='white');sheet.save(out/'comparison.png')
