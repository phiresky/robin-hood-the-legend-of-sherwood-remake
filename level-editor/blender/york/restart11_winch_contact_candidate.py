"""Create modest source-led support/wire corrections with flat closed support feet."""
import hashlib,json,math,sys,runpy
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'winch-room-physical-v10'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,bmesh
from mathutils import Vector
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
c=math.cos(math.radians(35));floor=90.00101/c;OUT.mkdir();records=[]
for state in ('transition-00','transition-44'):
 source=WORK/'winch-room-physical-v9'/state/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
 supports=[o for o in scene.objects if o.name.startswith(('Angled','Frame foot rail'))];chains=[o for o in scene.objects if o.name.startswith('Suspended chain')];changed=set(supports+chains)
 protected={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in changed};caps=[]
 for o in chains:
  for v in o.data.vertices:
   old=v.co.copy();r=math.hypot(old.x,old.y);scale=(1.5+(r-1.5)*(.85/.75))/r;v.co=Vector((old.x*scale,old.y*scale,old.z*(.85/.75)))
  o.data.update()
 for o in supports:o.scale.x*=1.15;o.scale.y*=1.15
 bpy.context.view_layer.update()
 for o in supports:
  bm=bmesh.new();bm.from_mesh(o.data)
  for v in bm.verts:v.co=o.matrix_world@v.co
  result=bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=1e-6,plane_co=Vector((0,0,floor)),plane_no=Vector((0,0,1)),clear_inner=True,clear_outer=False)
  edges=[e for e in bm.edges if e.is_boundary];assert edges,o.name;bmesh.ops.holes_fill(bm,edges=edges,sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges),o.name
  contact=[v for v in bm.verts if abs(v.co.z-floor)<1e-5];assert len(contact)>=3
  caps.append({'object':o.name,'flat_foot_vertices':len(contact),'minimum_game_z':min(v.co.z for v in bm.verts)*c,'closed':True})
  inv=o.matrix_world.inverted()
  for v in bm.verts:v.co=inv@v.co
  bm.to_mesh(o.data);bm.free();o.data.update()
 bpy.context.view_layer.update();assert protected=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in protected}
 dest=OUT/state;dest.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True);records.append({'state':state,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((dest/'model.blend').read_bytes()).hexdigest(),'outside_objects_exact':len(protected),'foot_caps':caps})
(OUT/'proposal.json').write_text(json.dumps({'status':'Private HOLD pending native/all-view inspection','scope':'Support radii +15%; chain wire radius0.75→0.85. Each support bottom clipped and closed on known physical floor. All chain centres, crank and other scene objects retained.','basis':'Bounded source-opacity comparison includes extra source-empty pixels; larger1.05 chain radius rejected because it closes two authentic final openings.','states':records},indent=2)+'\n')
sys.argv=['restart2_winch_room_probe_audit.py','--','winch-room-physical-v10'];runpy.run_path(str(Path(__file__).with_name('restart2_winch_room_probe_audit.py')),run_name='__main__')
