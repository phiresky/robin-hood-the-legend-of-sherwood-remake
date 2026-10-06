"""Review a continuous native-radius lower volume before material grafting."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,SPECS
from restart6_tree39_contour import covered
from restart4_stump_final_contact import sheet
from render_views import render_views
from render_slots import acquire,release
from evidence_io import write_json,sha
acquire()
try:
 out=ROOT/'tree39-native-volume-v1';source=SPECS[39][0];bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;bpy.context.view_layer.update();wood=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-39'and 'Crown'not in o.name];item=next(r for r in json.load(open(OUT/'review-mask-inventory.json'))['masks']if r['index']==39);ox,oy=item['box_top_left'];yy,xx=np.where(np.array(Image.open(item['png']))>0);pts=np.column_stack((xx+ox+.5,yy+oy+.5));prior=covered(wood,pts);gap=np.array(Image.open(ROOT/'source-audit-v1/exposed-39.png'))>0;required=prior|gap[yy+oy,xx+ox]
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=True
 data=np.load(out/'lower.npz');mesh=bpy.data.meshes.new('Continuous native lower volume');mesh.from_pydata(data['vertices'].tolist(),[],data['faces'].tolist());mesh.update();lower=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(lower)
 for p in mesh.polygons:p.use_smooth=True
 original=next(o for o in wood if o.get('source_node')=='building-095');mesh=original.data.copy();mesh.transform(original.matrix_world);upper=bpy.data.objects.new('Exact upper support diagnostic',mesh);scene.collection.objects.link(upper);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=(0,0,100),plane_no=(0,0,1),dist=.0001,clear_inner=True);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bm.to_mesh(mesh);bm.free();bpy.context.view_layer.update();cov=covered([upper,lower],pts)
 info=json.load(open(ROOT/'tree39-solid-delta-v1/evidence.json'));views={};scene.render.resolution_x=512;scene.render.resolution_y=512;scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH'
 for i,c in enumerate(info['same_cameras']):
  cam=bpy.data.objects.new(f'Native lower {i}',bpy.data.cameras.new(f'Native lower {i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=c['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(c['matrix']);views[f'view-{i}']=cam.name
 render_views(scene.name,views,out/'solid',modes=('solid',),width=512);sheet([out/f'solid/view-{i}-solid.png'for i in range(4)],out/'sheet.png');write_json(out/'coverage.json',dict(approved_parent_sha256=sha(source),required=int(required.sum()),covered=int((required&cov).sum()),missing=(pts[required&~cov]-.5).tolist(),target_covered=int((cov&gap[yy+oy,xx+ox]).sum()),old_coverage_lost=int((prior&~cov).sum()),scope='Private SDF lower/native coverage test; no final geometry/texture decision.'))
finally:release()
