"""Fit one private leaf pile to a low wall toe along fixed native source rays."""
from pathlib import Path
import sys,json,hashlib
import bpy,bmesh
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from leaf_state_scene_context import load_scene
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def bvh(obj):
 obj.data.calc_loop_triangles();tri=[tuple(t.vertices)for t in obj.data.loop_triangles];points=[obj.matrix_world@v.co for v in obj.data.vertices]
 return BVHTree.FromPolygons(points,tri,all_triangles=True),tri,points

def main():
 root=OUT/'restart9-hiding-scatter';out=root/'mound-support-variants-v3';out.mkdir(exist_ok=False);parent=root/'mound-support-variants-v2';rec=json.loads((parent/'validation.json').read_text());assert sha(parent/'model.blend')==rec['model_sha256'];scene,static,pins,base=load_scene();wallverts=[];walltris=[];wallpins=[]
 for o in static:
  if o.get('asset_group')!='croisement02-southeast-stone-wall-and-gate':continue
  _,tris,points=bvh(o);start=len(wallverts);wallverts+=points;walltris +=[tuple(i+start for i in t)for t in tris];wallpins.append(dict(object=o.name,matrix_world=[list(r)for r in o.matrix_world]))
 wall=BVHTree.FromPolygons(wallverts,walltris,all_triangles=True)
 bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));bpy.context.view_layer.update();row=next(r for r in rec['records']if 'mission-Tac02_FoB_EC-patch-022'in r['instances']);obj=bpy.data.objects[row['object']];assert obj.matrix_world==Matrix.Identity(4)
 original=[v.co.copy()for v in obj.data.vertices];columns={}
 for i,p in enumerate(original):columns.setdefault((round(p.x,3),round(-p.y*SIN-p.z*COS,3)),[]).append(i)
 lookup={i:key for key,ids in columns.items()for i in ids};shifts={k:0. for k in columns};sampled=0
 for key,ids in columns.items():
  p=min((original[i]for i in ids),key=lambda p:p.z);q,normal,ti,d=wall.ray_cast(p+RAY*1000,-RAY)
  if q is not None and -.01<=q.z<=10:
   shifts[key]=max(0.,(q-p).dot(RAY)+.02);sampled+=1
 def apply():
  for key,ids in columns.items():
   assert shifts[key]*RAY.z<=4., 'Correction exceeds bounded wall-toe scope'
   for i in ids:obj.data.vertices[i].co=original[i]+RAY*shifts[key]
  obj.data.update()
 apply();history=[]
 for iteration in range(160):
  current,tris,_=bvh(obj);pairs=current.overlap(wall);history.append(len(pairs))
  if not pairs:break
  keys={lookup[i]for ti,_ in pairs for i in tris[ti]}
  for key in keys:shifts[key]+=.10
  apply()
 else:raise AssertionError('Finite contact correction did not converge')
 bm=bmesh.new();bm.from_mesh(obj.data);closed=all(e.is_manifold for e in bm.edges)and all(v.is_manifold for v in bm.verts);volume=bm.calc_volume(signed=True);bm.free();assert closed and volume>0
 error=max(max(abs(v.co.x-p.x),abs((-v.co.y*SIN-v.co.z*COS)-(-p.y*SIN-p.z*COS)))for v,p in zip(obj.data.vertices,original));assert error<.001
 row.update(closed=closed,volume=volume,maximum_projection_error=error,wall_support_correction=True)
 path=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(path));result={**rec,'model_sha256':sha(path),'parent_model_sha256':sha(parent/'model.blend'),'status':'PRIVATE_WALL_CONTACT_CORRECTION','wall_context':{'base_sha256':sha(base),'substitutions':pins,'objects':wallpins},'wall_contact':{'triangle_intersection_counts':history,'sampled_columns':sampled,'shifted_columns':sum(v>0 for v in shifts.values()),'total_columns':len(columns),'maximum_ray_shift':max(shifts.values()),'maximum_height_gain':max(shifts.values())*RAY.z,'source_projection_error':error},'scope':'Only Tac02 patch022 initial mound adapted to exact low wall toe. Other19 variants untouched. Fixed source rays and UVs retained; closed topology and zero wall intersections checked. Local underside bridging remains inferred support geometry.'};(out/'validation.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result['wall_contact']),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
