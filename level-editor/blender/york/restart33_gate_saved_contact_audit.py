"""Read-only saved gate/jamb mesh contact audit for the private motion proposal."""
import hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'gate-saved-contact-audit-v1';assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();source=W/'gate-textures-v1/covered/model.blend';digest=sha(source);assert digest=='6ed0064f493f79cb38f249fdbc41177eac3cee5659fb86c73f59ae746ad19b70';bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();gate=bpy.data.objects['scenery-york-castle-portcullis'];jamb=bpy.data.objects['building-778-portcullis-jamb-return'];gv=[gate.matrix_world@v.co for v in gate.data.vertices];gf=[tuple(p.vertices)for p in gate.data.polygons];jv=[jamb.matrix_world@v.co for v in jamb.data.vertices];jf=[tuple(p.vertices)for p in jamb.data.polygons];jt=BVHTree.FromPolygons(jv,jf);jambtree=jt;rows=[];proposal=json.loads((W/'gate-motion-proposal-v2/motion.json').read_text());direction=Vector((.1237,.2381,.9633)).normalized()
def inside(point):
 origin=point.copy();hits=0
 for _ in range(200):
  loc,n,idx,d=jt.ray_cast(origin,direction)
  if loc is None:return bool(hits%2)
  hits+=1;origin=loc+direction*.002
 raise RuntimeError('Parity ray failed to terminate')
for row in proposal['rows']:
 dz=row['nominal_lift_world_z'];vs=[v+Vector((0,0,dz))for v in gv];gt=BVHTree.FromPolygons(vs,gf);pairs=gt.overlap(jt);penetrating=[]
 for fi,face in enumerate(gf):
  center=sum((vs[i]for i in face),Vector())/len(face);nearest=jt.find_nearest(center);depth=nearest[3]
  if depth>.003 and inside(center):penetrating.append({'gate_face':fi,'depth_to_jamb_surface':depth,'center':list(center)})
 rows.append({'frame':row['frame'],'lift_source_pixels':row['nominal_lift_source_pixels'],'triangle_intersection_pairs':len(pairs),'pair_sample':pairs[:40],'gate_face_centers_inside_jamb':penetrating,'max_center_penetration':max((r['depth_to_jamb_surface']for r in penetrating),default=0),'gate_min_world_z':min(v.z for v in vs)})
assert sha(source)==digest
report={'status':'READ_ONLY_SAVED_MESH_CONTACT_AUDIT','model':str(source),'model_sha256':digest,'jamb_object':jamb.name,'jamb_material_scope':'Frozen covered model context; separately approved appearance does not change vertices','gate_vertices':len(gv),'gate_faces':len(gf),'jamb_vertices':len(jv),'jamb_faces':len(jf),'geometry_world':{'gate_vertices':[list(v)for v in gv],'gate_faces':gf,'jamb_vertices':[list(v)for v in jv],'jamb_faces':jf},'rows':rows,'limits':['BVH overlap detects triangle intersections; parity-tested face centers confirm some volumetric penetration but cannot rule out all edge-only penetration.','Guide/recess interpretation must be supported by actual saved geometry, not assumed from approved endpoint labels.','No model was changed or saved; native timing/art untouched.']};OUT.mkdir();(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file())<16*1024**2;print(json.dumps({'out':str(OUT),'endpoint0':{k:rows[0][k]for k in ['triangle_intersection_pairs','max_center_penetration']},'endpoint44':{k:rows[-1][k]for k in ['triangle_intersection_pairs','max_center_penetration']},'maximum_penetration':max(r['max_center_penetration']for r in rows)}))
