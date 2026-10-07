"""Review complete shaft return and enlarged body-entry geometry, native first."""
import json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-shaft-return-candidate-v1';OUT=BASE/'review'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0,str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest,labeled_copy
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();OUT.mkdir();own=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004']
for o in scene.objects:
 if o.type=='MESH':o.hide_render=o not in own
for label,targets in [('complete',own),('body-entry',[o for o in own if not o.name.startswith(('Shaft return','Inferred upper','Travelling round'))])]:
 points=[o.matrix_world@v.co for o in targets for v in o.data.vertices];lo=Vector(tuple(min(p[i] for p in points) for i in range(3)));hi=Vector(tuple(max(p[i] for p in points) for i in range(3)));center=(lo+hi)/2;views={};rows=[]
 for i in range(8):
  cam=scene.objects[f'Winch{i}'];toward=cam.matrix_world.to_quaternion()@Vector((0,0,1));cam.location=center+toward*10000;r=cam.matrix_world.to_quaternion().inverted();pp=[r@(p-center) for p in points];bpy.context.view_layer.update();cam.data.ortho_scale=max(max(p.y for p in pp)-min(p.y for p in pp),(max(p.x for p in pp)-min(p.x for p in pp))*384/320)*1.35;views[f'view-{i}']=cam.name;rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cam.matrix_world],'ortho_scale':cam.data.ortho_scale})
 bpy.context.view_layer.update();folder=OUT/label;folder.mkdir();(folder/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(folder/'views.json');scene.render.resolution_x=320;scene.render.resolution_y=384;render_views(scene.name,views,folder/'renders',modes=('textured','solid'),width=320)
 for mode in ('textured','solid'):
  sheet=Image.new('RGBA',(1280,768))
  for i in range(8):sheet.paste(Image.open(folder/f'renders/view-{i}-{mode}.png'),((i%4)*320,(i//4)*384))
  sheet.save(folder/f'{mode}8.png');labeled_copy(folder/f'{mode}8.png',folder/f'{mode}8-native-labeled.png')
for o in scene.objects:
 if o.type=='MESH':o.hide_render=False
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));cam=scene.objects['Winch0'];cam.location=Vector((2410,-930/s,0))+back*10000;cam.data.ortho_scale=110;scene.render.resolution_x=180;scene.render.resolution_y=330;render_views(scene.name,{'native':cam.name},OUT/'native',modes=('textured',),width=180)
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');f=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')[44];tile=Image.new('RGBA',(60,110),'#303840');tile.alpha_composite(Image.open(source/f['image']).convert('RGBA'),(f['bbox'][0]-2380,f['bbox'][1]-875));sheet=Image.new('RGB',(360,354),'#303840');sheet.paste(tile.resize((180,330),Image.Resampling.NEAREST).convert('RGB'),(0,24));im=Image.open(OUT/'native/native-textured.png');sheet.paste(im,(180,24),im);ImageDraw.Draw(sheet).text((4,5),'Original / private shaft return',fill='white');sheet.save(OUT/'native-comparison.png');print('SHAFT RETURN REVIEW SAVED')
