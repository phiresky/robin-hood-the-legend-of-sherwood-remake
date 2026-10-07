"""Fit the lower frame support endpoints to visible timber without changing the axle."""
import hashlib,json,math,sys,runpy
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'winch-room-physical-v9'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
s,c=math.sin(math.radians(35)),math.cos(math.radians(35))
def world(x,y,z):return Vector((x,-y/s,z/c))
# Source endpoint interpretation is uncertain by about two native pixels. The
# rear frame has upright left support and an outward right support, rather
# than duplicating the front A frame. Feet retain the independently known floor.
poses={
 'Angled left frame brace.001':((2398,1060,90.00101),(2398,1064,113)),
 'Angled right frame brace.001':((2417,1060,90.00101),(2405,1064,113)),
 'Frame foot rail.001':((2397,1060,91),(2418,1060,91)),
}
OUT.mkdir();records=[]
for state in ('transition-00','transition-44'):
 source=WORK/'winch-room-physical-v8'/state/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
 protected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name not in poses}
 for name,(a,b) in poses.items():
  o=scene.objects[name];a,b=world(*a),world(*b)
  zlo=min(v.co.z for v in o.data.vertices);zhi=max(v.co.z for v in o.data.vertices)
  o.location=(a+b)/2;o.rotation_euler=(b-a).to_track_quat('Z','Y').to_euler();o.scale.z=(b-a).length/(zhi-zlo)
 bpy.context.view_layer.update();assert protected=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in protected}
 dest=OUT/state;dest.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
 records.append({'state':state,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((dest/'model.blend').read_bytes()).hexdigest(),'outside_objects_exact':len(protected)})
(OUT/'proposal.json').write_text(json.dumps({'status':'PRIVATE diagnostic hypothesis, no geometry approval','changed_components':poses,'basis':'Lower rear-left upright timber near native2398,958 through2398,970 and outward right support ending2417,970; full source v8 body axes reviewed. Two-pixel endpoint uncertainty. Hidden depth inferred; floor90.00101 retained.','states':records},indent=2)+'\n')
sys.argv=['restart2_winch_room_probe_audit.py','--','winch-room-physical-v9'];runpy.run_path(str(Path(__file__).with_name('restart2_winch_room_probe_audit.py')),run_name='__main__')
