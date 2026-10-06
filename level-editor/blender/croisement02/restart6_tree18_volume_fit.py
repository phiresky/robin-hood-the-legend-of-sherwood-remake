"""Review a continuous native-radius lower volume before material grafting."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_tree18_contour import ROOT,OUT,SPECS
from restart6_tree39_contour import covered,projected_tree,projection
from mathutils.geometry import barycentric_transform
from restart6_source_gap_audit import RAY,SIN,COS
from restart4_stump_final_contact import sheet,frame
from render_views import render_views
from render_slots import acquire,release
from evidence_io import write_json,sha
acquire()
try:
 out=ROOT/'tree18-native-volume-v2';out.mkdir(exist_ok=False);source=SPECS[18][0];bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;bpy.context.view_layer.update();wood=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-18'and 'Crown'not in o.name];item=next(r for r in json.load(open(OUT/'review-mask-inventory.json'))['masks']if r['index']==18);ox,oy=item['box_top_left'];yy,xx=np.where(np.array(Image.open(item['png']))>0);pts=np.column_stack((xx+ox+.5,yy+oy+.5));prior=covered(wood,pts);gap=np.array(Image.open(ROOT/'source-audit-v1/exposed-18.png'))>0;required=prior|gap[yy+oy,xx+ox]
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=True
 data=np.load(ROOT/'tree18-native-volume-v1/lower.npz');mesh=bpy.data.meshes.new('Continuous native lower volume');mesh.from_pydata(data['vertices'].tolist(),[],data['faces'].tolist());mesh.update();lower=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(lower)
 for p in mesh.polygons:p.use_smooth=True
 original=next(o for o in wood if o.get('source_node')=='building-101');mesh=original.data.copy();mesh.transform(original.matrix_world);upper=bpy.data.objects.new('Exact upper support diagnostic',mesh);scene.collection.objects.link(upper);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=(0,0,100),plane_no=(0,0,1),dist=.0001,clear_inner=True);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bm.to_mesh(mesh);bm.free();bpy.context.view_layer.update();cov=covered([upper,lower],pts)
 targets=pts[required];history=[];before=np.array([v.co[:]for v in lower.data.vertices])
 for iteration in range(18):
  cov=covered([upper,lower],targets);history.append(dict(iteration=iteration,covered=int(cov.sum()),required=len(targets)));print(history[-1],flush=True)
  if cov.all():break
  tree,verts,world,faces,owners=projected_tree([lower]);p=np.array([v.co[:]for v in lower.data.vertices]);q=projection(p);sums=np.zeros((len(p),2));weights=np.zeros(len(p));ray=np.array(RAY)
  for target in targets[~cov]:
   point,n,i,d=tree.find_nearest(Vector((*target,0)));ids=faces[i];anchor=np.array(barycentric_transform(point,*[verts[j]for j in ids],*[world[j]for j in ids]));delta=target-np.array(point[:2]);distance=np.linalg.norm(delta)
   if distance<1e-4:continue
   delta*=1+.65/distance;radius=max(8.,distance*2+5);dq=np.linalg.norm(q-np.array(point[:2]),axis=1);depth=abs((p-anchor)@ray);w=np.exp(-2*(dq/radius)**2);w[(dq>radius*2.5)|(p[:,2]>100)]=0;sums+=w[:,None]*delta;weights+=w
  shift=sums/(1+weights[:,None]);p+=np.column_stack((shift[:,0],-SIN*shift[:,1],-COS*shift[:,1]));low=p[:,2]<.15;p[low,1]-=(.15-p[low,2])*COS/SIN;p[low,2]=.15
  for v,point in zip(lower.data.vertices,p):v.co=point
  lower.data.update();bpy.context.view_layer.update()
 cov=covered([upper,lower],pts);np.savez_compressed(out/'lower.npz',vertices=np.array([v.co[:]for v in lower.data.vertices]),faces=data['faces']);write_json(out/'fit.json',dict(history=history,max_movement=float(np.linalg.norm(p-before,axis=1).max()),unchanged_above_z100=True))
 camera_records=[]
 for direction in [RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]:
  cam=frame(scene,[lower],direction.normalized(),512,1.25);camera_records.append(dict(matrix=[list(r)for r in cam.matrix_world],scale=cam.data.ortho_scale))
 info={'same_cameras':camera_records};views={};scene.render.resolution_x=512;scene.render.resolution_y=512;scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.65,.65,.65);scene.display.shading.show_cavity=True;scene.display.shading.cavity_type='BOTH'
 for i,c in enumerate(info['same_cameras']):
  cam=bpy.data.objects.new(f'Native lower {i}',bpy.data.cameras.new(f'Native lower {i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=c['scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(c['matrix']);views[f'view-{i}']=cam.name
 render_views(scene.name,views,out/'solid',modes=('solid',),width=512);sheet([out/f'solid/view-{i}-solid.png'for i in range(4)],out/'sheet.png');write_json(out/'coverage.json',dict(approved_parent_sha256=sha(source),required=int(required.sum()),covered=int((required&cov).sum()),missing=(pts[required&~cov]-.5).tolist(),target_covered=int((cov&gap[yy+oy,xx+ox]).sum()),old_coverage_lost=int((prior&~cov).sum()),scope='Private SDF lower/native coverage test; no final geometry/texture decision.'))
finally:release()
