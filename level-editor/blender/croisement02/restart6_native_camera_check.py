"""Verify saved native projection without inherited camera markers or borders."""
import sys,json
from pathlib import Path
import bpy
from mathutils import Matrix
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT
from render_slots import acquire,release
from render_views import render_views
from evidence_io import write_json
acquire()
try:
 out=ROOT/'tree39-native-camera-check-v3';out.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(ROOT/'tree39-contour-v4/model.blend'));scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render='Crown'in o.name
 for o in scene.objects:
  if o.type!='MESH'or 'Crown'in o.name:continue
  overlays=[i for i,m in enumerate(o.data.materials)if m and 'lower contour exact native'in m.name]
  if not overlays:continue
  for face in o.data.polygons:
   c=o.matrix_world@face.center
   if -c.y*.573576436351-c.z*.819152044289>565:face.material_index=overlays[0]
 info=json.loads((ROOT/'baseline-audit-39-v2/camera.json').read_text());cam=bpy.data.objects.new('Explicit native check',bpy.data.cameras.new('Explicit native check'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=info['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(info['camera']);scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
 rows=[]
 bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get()
 for o in scene.objects:
  if o.type!='MESH':continue
  o.data.calc_loop_triangles();ev=o.evaluated_get(deps);mesh=ev.to_mesh();mesh.calc_loop_triangles();rows.append(dict(name=o.name,raw=len(o.data.loop_triangles),evaluated=len(mesh.loop_triangles),triangle_mismatches=sum(tuple(a.vertices)!=tuple(b.vertices) or a.polygon_index!=b.polygon_index or a.material_index!=b.material_index for a,b in zip(o.data.loop_triangles,mesh.loop_triangles))));ev.to_mesh_clear()
 write_json(out/'triangles.json',rows)
 write_json(out/'settings.json',dict(markers=[dict(frame=m.frame,camera=m.camera.name if m.camera else None)for m in scene.timeline_markers],border=scene.render.use_border,aspect=[scene.render.pixel_aspect_x,scene.render.pixel_aspect_y]))
 render_views(scene.name,{'native':cam.name},out/'render',modes=('textured',),width=512)
finally:release()
