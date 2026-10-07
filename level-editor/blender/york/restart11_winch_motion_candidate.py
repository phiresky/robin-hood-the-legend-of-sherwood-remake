"""Author a private complete-geometry winch motion from source-measured poses."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'winch-motion-physical-v2'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector,Quaternion
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
c=math.cos(math.radians(35));s=math.sin(math.radians(35))
def world(x,y,z):return Vector((x,-y/s,z/c))
source=WORK/'winch-room-physical-v10/transition-44/model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
measure_path=WORK/'winch-motion-measurement-v2/measurement.json';measure=json.loads(measure_path.read_text());fit_path=WORK/'winch-crank-motion-fit-v1/fit.json';fit=json.loads(fit_path.read_text());assert hashlib.sha256(source.read_bytes()).hexdigest()==fit['source_model_sha256']
spokes=sorted((o for o in scene.objects if o.name.startswith('Crank spoke')),key=lambda o:o.name);travelling=[o for o in scene.objects if o.name.startswith('Travelling round part')];assert len(spokes)==8 and len(travelling)==6
protected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in spokes+travelling};center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized()
crank=bpy.data.objects.new('Winch crank physical rotation',None);scene.collection.objects.link(crank);crank.location=center;crank.rotation_mode='QUATERNION';travel=bpy.data.objects.new('Winch travelling part physical descent',None);scene.collection.objects.link(travel);bpy.context.view_layer.update()
for parent,children in ((crank,spokes),(travel,travelling)):
 for o in children:
  saved=o.matrix_world.copy();o.parent=parent;o.matrix_parent_inverse=parent.matrix_world.inverted();o.matrix_world=saved
 parent['source_node']='scenery-york-castle-winch';parent['asset_group']='york-castle-winch';parent['native_patch']='patch-004'
rows=[];coef=measure['descent_fit_diagnostic']['quadratic_descending_coefficients'];tick=0
for m,f in zip(measure['rows'],fit['frames']):
 i=m['frame'];assert i==f['frame'];inferred=m['observed_center_y'] is None;y=m['observed_center_y'] if not inferred else coef[0]*i*i+coef[1]*i+coef[2]
 phase=f['unwrapped_phase_degrees'];crank.rotation_quaternion=Quaternion(axis,-math.radians(phase+10));travel.location.z=(925-y)/c
 crank.keyframe_insert('rotation_quaternion',frame=tick);travel.keyframe_insert('location',frame=tick)
 rows.append({'source_frame':i,'tick':tick,'source_delay':m['delay'],'phase_degrees':phase,'screen_center_y':y,'descent_world_z':travel.location.z,'early_center_inferred':inferred});tick+=m['delay']+1
for o in (crank,travel):
 if o==crank:o.keyframe_insert('rotation_quaternion',frame=tick)
 else:o.keyframe_insert('location',frame=tick)
 action=o.animation_data.action
 for layer in action.layers:
  for strip in layer.strips:
   bag=strip.channelbag(o.animation_data.action_slot)
   if bag:
    for fc in bag.fcurves:
     for key in fc.keyframe_points:key.interpolation='CONSTANT'
scene.frame_start=0;scene.frame_end=tick;scene.render.fps=25;scene.frame_set(0);bpy.context.view_layer.update()
assert protected=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in protected}
# Verify actual evaluated spoke orientation, including axis handedness.
lateral=axis.cross(Vector((0,0,1))).normalized()
for r in rows:
 scene.frame_set(r['tick']);bpy.context.view_layer.update();o=spokes[0];lo=min(v.co.z for v in o.data.vertices);hi=max(v.co.z for v in o.data.vertices);direction=(o.matrix_world@Vector((0,0,hi))-o.matrix_world@Vector((0,0,lo))).normalized();actual=math.degrees(math.atan2(direction.z,direction.dot(lateral)));error=(actual-r['phase_degrees']+180)%360-180;assert abs(error)<.01,(r['source_frame'],actual,r['phase_degrees']);r['actual_spoke_phase_error_degrees']=error
scene.frame_set(0);bpy.context.view_layer.update()
OUT.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
(OUT/'motion.json').write_text(json.dumps({'status':'Private motion hypothesis; requires every-frame source/occlusion and saved-model review','source_model_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'measurement_sha256':hashlib.sha256(measure_path.read_bytes()).hexdigest(),'crank_fit_sha256':hashlib.sha256(fit_path.read_bytes()).hexdigest(),'rows':rows,'duration_ticks':tick,'tick_rate_hz':25,'interpolation':'CONSTANT, native frame hold','outside_geometry_appearance_exact':len(protected),'limitations':['Initial state is transparent and separate from visible transition00; state integration not authored.','All six travelling part meshes stay whole behind the room during early frames; early centre trajectory is inferred.','Crank has eightfold silhouette ambiguity; source fitting resolves a smooth phase hypothesis rather than unique spoke identity.','Chain link motion and native sound360 remain unimplemented.','Source materials are still unknown diagnostic colors; no texture approval or library publication.']},indent=2)+'\n')
print('WINCH PHYSICAL MOTION SAVED',tick)
