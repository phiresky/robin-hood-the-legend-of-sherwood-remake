"""Inspect native contact plus unseen gate undersides on saved texture candidates."""
import sys,math,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
for state,src in [('jamb-textures-v1','initial')]:
 out=BASE/state/'contact-review';out.mkdir();bpy.ops.wm.open_mainfile(filepath=str(BASE/state/'model.blend'));scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=False
 scene.render.resolution_x=440;scene.render.resolution_y=500;render_views(scene.name,{'native':'Native crop'},out/'native',modes=('textured',),width=440)
 bg=Image.open(WORK/f'restart2/gate-source-study-v2/{src}.png').convert('RGB').resize((440,500),Image.Resampling.NEAREST);a=Image.open(out/'native/native-textured.png');sheet=Image.new('RGB',(880,524),'#303840');sheet.paste(bg,(0,24));sheet.paste(a,(440,24),a);d=ImageDraw.Draw(sheet);d.text((5,5),'Native artwork',fill='white');d.text((445,5),'Jamb texture candidate; other context unfinished',fill='white');sheet.save(out/'native-comparison.png')
 gate=bpy.data.objects['building-778-portcullis-jamb-return'];points=[gate.matrix_world@v.co for v in gate.data.vertices];center=sum(points,Vector())/len(points);names={};rows=[]
 for i,(yaw,elev) in enumerate([(0,35),(0,-35),(90,-35),(180,-35),(270,-35)]):
  a,b=math.radians(yaw),math.radians(elev);back=Vector((math.sin(a)*math.cos(b),-math.cos(a)*math.cos(b),math.sin(b)));data=bpy.data.cameras.new('Underside'+str(i));cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);data.type='ORTHO';data.ortho_scale=180;data.clip_end=20000;cam.location=center+back*10000;cam.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();names[f'view-{i}']=cam.name;rows.append({'view':i,'yaw':yaw,'elevation':elev})
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o!=gate
 scene.render.resolution_x=320;scene.render.resolution_y=384;render_views(scene.name,names,out/'underside',modes=('textured',),width=320);sheet=Image.new('RGB',(960,808),'#303840');draw=ImageDraw.Draw(sheet)
 for i,r in enumerate(rows):
  x,y=(i%3)*320,(i//3)*404;a=Image.open(out/f'underside/view-{i}-textured.png');sheet.paste(a,(x,y+20),a);draw.text((x+4,y+4),'Native game camera' if i==0 else f'Underside yaw {r["yaw"]}',fill='white')
 sheet.save(out/'native-and-undersides.png');(out/'views.json').write_text(json.dumps(rows,indent=2)+'\n')
