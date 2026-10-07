"""Private source-led winch components; no runtime coupling or source material assignment."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';OUT=WORK/'restart2/winch-geometry-v5'
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
source=WORK/'restart2/gate-geometry-v10/covered/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
for o in list(bpy.data.objects):
 if o.type in {'CAMERA','LIGHT'} or o.get('native_patch')=='patch-000':bpy.data.objects.remove(o,do_unlink=True)
context=[o for o in scene.objects if o.type=='MESH'];s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
def world(x,y,z):return Vector((x,-y/s,z/c))
def mat(name,col):
 m=bpy.data.materials.new(name);m.use_nodes=True;m.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(*col,1);return m
wood=mat('Unknown winch timber',(.38,.22,.10));iron=mat('Unknown winch iron',(.10,.12,.13));parts=[];moving=[];crank=[]
def own(o,name,m):
 o.name=name;o['source_node']='scenery-york-castle-winch';o['asset_group']='york-castle-winch';o['native_patch']='patch-004';o.data.materials.clear();o.data.materials.append(m);parts.append(o);return o

def beam(name,a,b,r,m=wood):
 a,b=Vector(a),Vector(b);bpy.ops.mesh.primitive_cylinder_add(vertices=8,radius=r,depth=(b-a).length,location=(a+b)/2);o=own(bpy.context.object,name,m);o.rotation_euler=(b-a).to_track_quat('Z','Y').to_euler();return o
# Floor is obstacle98 at90.00101. Frame footprint and hidden depth are inferred.
for y in (1055,1064):
 top=110.5 if y==1055 else 113
 beam('Angled left frame brace',world(2391,y,90.00101),world(2398,y,top),2.2)
 beam('Angled right frame brace',world(2412,y,90.00101),world(2405,y,top),2.2)
 beam('Frame top saddle',world(2397,y,top),world(2407,y,top),2.0)
 beam('Frame foot rail',world(2390,y,91),world(2413,y,91),1.7)
center=world(2410,1064,104);rear=world(2402,1050,104);axle=(center-rear).normalized();lateral=axle.cross(Vector((0,0,1))).normalized()
beam('Broad timber winding drum',rear,center,4.3)
beam('Crank axle pin',rear-axle*2,center+axle*2,1.8,iron)
for i in range(8):
 angle=i*math.tau/8;end=center+(lateral*math.cos(angle)+Vector((0,0,1))*math.sin(angle))*17;crank.append(beam('Crank spoke',center,end,.9))
# Native enclosed openings constrain front-link centers. Link depth and
# alternate edge-facing links are inferred, with source holes kept explicit.
for y,centers,base_x in ((1059,[-43.5,-36.5,-29.5,-22.5,-15.5,-8.5,-1.5,5.5,12.5,19.5,26.5,33.5,40.5,47.5,54.5,60.5,67.5],2400.5),
                       (1064,[-41.5,-34.5,-27.5,-20.5,-13.5,-6.5,.5,7.5,14.5,21,27.5,34,40,46.5,53.5,60.5,67.5,74.5],2410.0)):
 entries=[(v,False) for v in centers]+[((a+b)/2,True) for a,b in zip(centers,centers[1:])]
 for sy,edge in entries:
  x=base_x+(0.5 if y==1059 else 1.0)*max(0,min(1,(sy-28)/25))
  z=y-(882+sy)
  bpy.ops.mesh.primitive_torus_add(major_segments=16,minor_segments=6,major_radius=1.45,minor_radius=.55,location=world(x,y,z))
  o=own(bpy.context.object,'Suspended chain link',iron)
  # Torus local XY becomes vertical; its smaller edge profile leaves the
  # adjacent front-facing opening visible from the native camera.
  o.scale.y=1.2 if edge else 1.4
  o.rotation_euler=(math.pi/2,0,math.pi/2 if edge else 0)
# The descending round part is preserved as a full ring with crossed spokes.
wheel_center=world(2398,1059,206)
bpy.ops.mesh.primitive_torus_add(major_segments=20,minor_segments=6,major_radius=7.5,minor_radius=1.3,location=wheel_center);o=own(bpy.context.object,'Travelling round part rim',wood);o.rotation_euler.x=math.pi/2;moving.append(o)
moving.append(beam('Travelling round part solid drum',wheel_center-Vector((0,2,0)),wheel_center+Vector((0,2,0)),7.4))
for i in range(4):
 a=i*math.pi/4;v=Vector((math.cos(a)*7.5,0,math.sin(a)*7.5));moving.append(beam('Travelling round part spoke',wheel_center-v,wheel_center+v,1))
scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.film_transparent=True
ld=bpy.data.lights.new('Winch review sun','SUN');lo=bpy.data.objects.new(ld.name,ld);scene.collection.objects.link(lo);lo.rotation_euler=(.5,-.6,-.4);ld.energy=2
OUT.mkdir(parents=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def cameras(center,scale,prefix):
 names={};rows=[]
 for i in range(8):
  yaw=i*math.pi/4;back=Vector((math.sin(yaw)*c,-math.cos(yaw)*c,s));d=bpy.data.cameras.new(prefix+str(i));o=bpy.data.objects.new(d.name,d);scene.collection.objects.link(o);d.type='ORTHO';d.ortho_scale=scale;d.clip_end=20000;o.location=center+back*10000;o.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();names[f'view-{i}']=o.name;rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in o.matrix_world],'ortho_scale':scale})
 return names,rows
iso,ir=cameras(world(2408,1068,156),200,'Winch');joint,jr=cameras(world(2377,1040,145),310,'Contact')
def review(p,names,rows):
 p.mkdir();(p/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(p/'views.json');scene.render.resolution_y=384;render_views(scene.name,names,p/'renders',modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
 for i in range(8):sheet.paste(Image.open(p/f'renders/view-{i}-textured.png'),((i%4)*320,(i//4)*384))
 sheet.save(p/'solid8.png');labeled_copy(p/'solid8.png',p/'solid8-native-labeled.png')
records=[]
for state,drop,show in [('transition-00',0,True),('transition-44',72,True)]:
 p=OUT/state;p.mkdir()
 for o in moving:o.hide_render=not show;o.location.z-=drop/c
 bpy.context.view_layer.update();bpy.ops.wm.save_as_mainfile(filepath=str(p/'model.blend'),compress=True)
 for o in context:o.hide_render=True
 review(p/'isolated',iso,ir)
 for o in context:o.hide_render=False
 review(p/'contact',joint,jr)
 records.append({'state':state,'model_sha256':sha(p/'model.blend')})
(OUT/'proposal.json').write_text(json.dumps({'status':'HOLD first geometry hypothesis, needs native comparison and self-review','states':records,'source_sha256':sha(source),'floor':{'obstacle':98,'sector':106,'layer':2,'height':90.00101},'scope':'Winch frame, rotating crank, two chains, separately descending round part. No patch000 coupling.','inferences':['Hidden frame depth and spoke plane inferred; source tests pending.','Transparent initial sprite has no mesh in initial appearance; transition00 is not initial state.','Early round part height206 and upper chain continuation are inferred behind the arch; native22/33/44 positions constrain subsequent descent. Both endpoints keep complete geometry; motion interpolation and upper anchor contacts remain pending.','Other gatehouse and floor proxies remain unrefined context.']},indent=2)+'\n');print('WINCH CANDIDATE COMPLETE',flush=True)
