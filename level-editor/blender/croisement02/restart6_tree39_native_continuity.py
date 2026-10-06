"""Restore coherent observed native RGB on the revised lower front only."""
import sys,json,shutil
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,RAY,SIN,COS,OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
acquire()
try:
 parent=ROOT/'tree39-contour-v4';out=ROOT/'tree39-contour-v6';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));bpy.context.view_layer.update();counts={}
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or o.get('source_node')not in ['building-095','building-096']or 'Crown'in o.name:continue
  overlays=[j for j,m in enumerate(o.data.materials)if m and 'lower contour exact native'in m.name];assert len(overlays)==1;slots={2:overlays[0]};n=0
  for face in o.data.polygons:
   if face.material_index not in slots:continue
   center=o.matrix_world@face.center;normal=o.matrix_world.to_3x3().inverted().transposed()@face.normal
   if -center.y*SIN-center.z*COS<=565 or normal.dot(RAY)<=0:continue
   newslot=slots[face.material_index]
   if newslot is None:continue
   face.material_index=newslot;n+=1
  counts[o.name]=n
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
 for name in ['fit.json','native39.png']:shutil.copy2(parent/name,out/name)
 write_json(out/'candidate.json',dict(model_sha256=sha(out/'model.blend'),geometry_parent_sha256=sha(parent/'model.blend'),extra_native_front_faces=counts,scope='Exact own native image/ownership alpha on continuous lower front-facing faces; retained fallback outside source ownership. Geometry unchanged from v4; crown and upper support unchanged.'))
finally:release()
