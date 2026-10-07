"""Review source-projected components separately from unapproved chain/context."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-components-source-v2';OUT=BASE/'review-v2'
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
freeze=json.loads((BASE/'component-freeze.json').read_text());assert hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest()==freeze['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scope=set(freeze['scope']);OUT.mkdir();images=[]
def orbit(out,names,modes):
 out.mkdir();rows=[]
 for i,name in enumerate(names.values()):
  cam=scene.objects[name];rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cam.matrix_world],'ortho_scale':cam.data.ortho_scale})
 (out/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(out/'views.json');scene.render.resolution_x=320;scene.render.resolution_y=384;render_views(scene.name,names,out/'renders',modes=modes,width=320)
 for mode in modes:
  sheet=Image.new('RGBA',(1280,768))
  for i in range(8):
   im=Image.open(out/f'renders/view-{i}-{mode}.png');assert im.size==(320,384);sheet.paste(im,((i%4)*320,(i//4)*384))
  path=out/f'{mode}8.png';sheet.save(path);labeled_copy(path,out/f'{mode}8-native-labeled.png');images.append(str(out/f'{mode}8-native-labeled.png'))
for index in (0,22,36,44):
 scene.frame_set(freeze['poses'][index]['tick']);bpy.context.view_layer.update()
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.name not in scope
 points=[o.matrix_world@v.co for o in scene.objects if o.name in scope for v in o.data.vertices];lo=Vector(tuple(min(p[a] for p in points) for a in range(3)));hi=Vector(tuple(max(p[a] for p in points) for a in range(3)));center=(lo+hi)/2
 for i in range(8):
  cc=scene.objects[f'Winch{i}'];toward=cc.matrix_world.to_quaternion()@Vector((0,0,1));cc.location=center+toward*10000;rotation=cc.matrix_world.to_quaternion().inverted();projected=[rotation@(p-center) for p in points];width=max(p.x for p in projected)-min(p.x for p in projected);height=max(p.y for p in projected)-min(p.y for p in projected);cc.data.ortho_scale=max(height,width*384/320)*1.25
 bpy.context.view_layer.update()
 orbit(OUT/f'components-{index:02d}',{f'view-{i}':f'Winch{i}' for i in range(8)},('textured','solid'))
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));d=bpy.data.cameras.new('Scoped native winch');cam=bpy.data.objects.new(d.name,d);scene.collection.objects.link(cam);d.type='ORTHO';d.ortho_scale=110;d.clip_end=20000;cam.location=Vector((2410,-930/s,0))+back*10000;cam.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler()
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')
sheet=Image.new('RGB',(1080,730),'#303840')
for j,index in enumerate((0,22,30,31,36,44)):
 scene.frame_set(freeze['poses'][index]['tick']);bpy.context.view_layer.update()
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=False
 scene.render.resolution_x=180;scene.render.resolution_y=330;render_views(scene.name,{'native':cam.name},OUT/f'native-{index:02d}',modes=('textured',),width=180)
 f=frames[index];tile=Image.new('RGBA',(60,110),'#303840');tile.alpha_composite(Image.open(source/f['image']).convert('RGBA'),(f['bbox'][0]-2380,f['bbox'][1]-875));tile=tile.resize((180,330),Image.Resampling.NEAREST);candidate=Image.open(OUT/f'native-{index:02d}/native-textured.png');x=(j%3)*360;y=(j//3)*365;sheet.paste(tile.convert('RGB'),(x,y+30));sheet.paste(candidate,(x+180,y+30),candidate);ImageDraw.Draw(sheet).text((x+4,y+5),f'{index}: native / source-projected components',fill='white')
sheet.save(OUT/'native-comparisons.png');images.append(str(OUT/'native-comparisons.png'))
scene.frame_set(freeze['poses'][44]['tick']);bpy.context.view_layer.update()
orbit(OUT/'contact-final',{f'view-{i}':f'Contact{i}' for i in range(8)},('textured',))
(OUT/'render-evidence.json').write_text(json.dumps({'model_sha256':freeze['model_sha256'],'images':{p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in images},'scope':freeze['scope'],'context':'Chain and room shown in native/contact evidence only, excluded from component scope.'},indent=2)+'\n')
print('WINCH COMPONENT REVIEW RENDERS SAVED')
