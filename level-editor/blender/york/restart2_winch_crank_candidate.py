"""Inspect source-balanced crank endpoint poses while protecting the room and frame."""
import hashlib
import json
import math
from pathlib import Path
import runpy
import sys
ROOT=Path(__file__).resolve().parents[3]
WORK=ROOT/'level-editor/work/york-refinement/restart2'
OUT=WORK/'winch-room-physical-v8'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
from render_views import render_views
sys.path.insert(0,str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest,labeled_copy
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
def world(x,y,z):return Vector((x,-y/s,z/c))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();lateral=axis.cross(Vector((0,0,1))).normalized()
OUT.mkdir();records=[]
for state,phase in [('transition-00',5),('transition-44',-10)]:
 source=WORK/'winch-room-physical-v7'/state/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
 spokes=sorted((o for o in scene.objects if o.name.startswith('Crank spoke')),key=lambda o:o.name);assert len(spokes)==8
 protected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in spokes}
 for i,o in enumerate(spokes):
  angle=i*math.tau/8+math.radians(phase);end=center+(lateral*math.cos(angle)+Vector((0,0,1))*math.sin(angle))*17
  o.location=(center+end)/2;o.rotation_euler=(end-center).to_track_quat('Z','Y').to_euler();o.scale=Vector((1.5/.9,1.5/.9,1))
 bpy.context.view_layer.update();assert protected=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in protected}
 dest=OUT/state;dest.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
 records.append({'state':state,'phase_degrees':phase,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((dest/'model.blend').read_bytes()).hexdigest(),'outside_objects_exact':len(protected)})
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004'
 scene.render.resolution_y=384;names={f'view-{i}':f'Winch{i}' for i in range(8)};rows=[]
 for i in range(8):
  cam=bpy.data.objects[f'Winch{i}'];rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cam.matrix_world],'ortho_scale':cam.data.ortho_scale})
 manifest=dest/'views.json';manifest.write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(manifest)
 render_views(scene.name,names,dest/'isolated',modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
 for i in range(8):sheet.paste(Image.open(dest/f'isolated/view-{i}-textured.png'),((i%4)*320,(i//4)*384))
 sheet.save(dest/'solid8.png');labeled_copy(dest/'solid8.png',dest/'solid8-native-labeled.png')
(OUT/'proposal.json').write_text(json.dumps({'status':'Private HOLD pending actual review; full animation phase still unproven','scope':'Only eight crank spokes; fixed axis/radius17, thickness1.5; independently source-fitted endpoint phase. Room/frame/chains exact.','states':records},indent=2)+'\n')
for recipe,args in [('restart2_winch_room_probe_audit.py',['winch-room-physical-v8']),('restart2_winch_room_review.py',['winch-room-physical-v8']),('restart2_winch_room_review.py',['winch-room-physical-v8','transition-00'])]:
 sys.argv=[recipe,'--',*args];runpy.run_path(str(Path(__file__).with_name(recipe)),run_name='__main__')
print('CRANK CANDIDATE COMPLETE',flush=True)
