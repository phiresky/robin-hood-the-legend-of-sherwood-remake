"""Read-only lower wood construction and ground contact for trees19/25."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY
from restart4_stump_final_contact import frame,sheet
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 assert shutil.disk_usage(OUT).free>25*2**30
 for number,limit in[(19,40),(25,65)]:
  prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);assert sha(source)==prior['source_sha256'];out=ROOT/f'tree{number}-toe-research-v1';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();scene=bpy.context.scene;asset=f'croisement02-tree-{number}';wood=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset and o.get('projection_component')!='crown'];points=[];rows=[]
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in wood
  for o in wood:
   p=np.array([o.matrix_world@v.co for v in o.data.vertices]);ids=np.flatnonzero(p[:,2]<limit);points.extend(p[ids]);rows.append(dict(object=o.name,source_node=o.get('source_node'),matrix=[list(r)for r in o.matrix_world],lower_vertices=[dict(index=int(i),world=p[i].tolist())for i in ids],faces=[list(f.vertices)for f in o.data.polygons]))
  write_json(out/'lower-mesh.json',dict(source=str(source),source_sha256=sha(source),height_limit=limit,objects=rows));mesh=bpy.data.meshes.new('Lower framing');mesh.from_pydata(points,[],[]);proxy=bpy.data.objects.new('Lower framing',mesh);scene.collection.objects.link(proxy);proxy.hide_render=True
  center=np.array(points).mean(0);bpy.ops.mesh.primitive_plane_add(size=180,location=(center[0],center[1],0));floor=bpy.context.object;floor.name='Z0 contact guide';mat=bpy.data.materials.new('Ground guide');mat.diffuse_color=(.12,.14,.1,1);floor.data.materials.append(mat)
  scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
  dirs=[RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]
  for i,d in enumerate(dirs):
   frame(scene,[proxy],d.normalized(),384,1.4);scene.render.filepath=str(out/f'contact-{i}.png');bpy.ops.render.render(write_still=True)
  sheet([out/f'contact-{i}.png'for i in range(4)],out/'contact4.png');assert sha(source)==prior['source_sha256']
finally:release()
