"""Native-first views of a separately scoped winch-room physical proposal."""
import json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2'/ (sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'winch-room-physical-v1');OUT=BASE/'review'
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
bpy.ops.wm.open_mainfile(filepath=str(BASE/'transition-44/model.blend'));scene=bpy.context.scene;OUT.mkdir();s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
for o in scene.objects:
 if o.type=='MESH':o.hide_render=False
names={};rows=[]
points=[o.matrix_world@v.co for o in scene.objects if o.type=='MESH' for v in o.data.vertices];low=Vector(tuple(min(p[i] for p in points) for i in range(3)));high=Vector(tuple(max(p[i] for p in points) for i in range(3)));center=(low+high)/2;scale=(high-low).length*1.2
for i in range(8):
 o=bpy.data.objects['Contact'+str(i)];yaw=i*math.pi/4;back=Vector((math.sin(yaw)*c,-math.cos(yaw)*c,s));o.location=center+back*10000;o.data.ortho_scale=scale;bpy.context.view_layer.update();names[f'view-{i}']=o.name;rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in o.matrix_world],'ortho_scale':scale})
(OUT/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(OUT/'views.json');scene.render.resolution_x=320;scene.render.resolution_y=384
render_views(scene.name,names,OUT/'renders',modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
for i in range(8):sheet.paste(Image.open(OUT/f'renders/view-{i}-textured.png'),((i%4)*320,(i//4)*384))
sheet.save(OUT/'contact8.png');labeled_copy(OUT/'contact8.png',OUT/'contact8-native-labeled.png')
d=bpy.data.cameras.new('Native room');o=bpy.data.objects.new(d.name,d);scene.collection.objects.link(o);d.type='ORTHO';d.ortho_scale=250;d.clip_end=20000;back=Vector((0,-c,s));o.location=Vector((2360,-905/s,0))+back*10000;o.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=440;scene.render.resolution_y=500
render_views(scene.name,{'native':o.name},OUT/'native',modes=('textured',),width=440)
original=Image.open(WORK/'restart2/gate-source-study-v2/transition-44.png').convert('RGB').resize((440,500),Image.Resampling.NEAREST);candidate=Image.open(OUT/'native/native-textured.png');sheet=Image.new('RGB',(880,524),'#303840');sheet.paste(original,(0,24));sheet.paste(candidate,(440,24),candidate);draw=ImageDraw.Draw(sheet);draw.text((5,5),'Native source (gate separate)',fill='white');draw.text((445,5),'Private room and winch hypothesis',fill='white');sheet.save(OUT/'native-comparison.png')
