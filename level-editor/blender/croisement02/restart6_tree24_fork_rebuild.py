"""Build a clean fork exterior and stitch it to retained original wood rings."""
import sys,math,json
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform,closest_point_on_tri
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT
from evidence_io import sha,write_json
from render_slots import acquire,release

def cut(mesh,z,lower):
 bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=(0,0,z),plane_no=(0,0,1),dist=.00001,clear_outer=lower,clear_inner=not lower);bm.to_mesh(mesh);bm.free();mesh.update()
def main():
 source=ROOT/'tree24-contour-v2/model.blend';trial=ROOT/'tree24-fork-union-v7/model.blend';out=ROOT/'tree24-fork-union-v9';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();obj=next(o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name);original=obj.data.copy();original.transform(obj.matrix_world);original.calc_loop_triangles();vertices=[v.co.copy()for v in original.vertices];faces=[tuple(t.vertices)for t in original.loop_triangles];surface=BVHTree.FromPolygons(vertices,faces,all_triangles=True);uvnames=[u.name for u in original.uv_layers];rows=[(t.material_index,{name:[Vector((*original.uv_layers[name].data[li].uv,0))for li in t.loops]for name in uvnames})for t in original.loop_triangles];materials=list(original.materials)
 with bpy.data.libraries.load(str(trial),link=False)as(src,dst):dst.objects=[n for n in src.objects if 'wood 124'in n]
 temp=dst.objects[0];bpy.context.scene.collection.objects.link(temp);bpy.context.view_layer.update();temp.data=temp.data.copy();temp.data.transform(temp.matrix_world);temp.matrix_world=Matrix.Identity(4);bpy.ops.object.select_all(action='DESELECT');temp.select_set(True);bpy.context.view_layer.objects.active=temp;modifier=temp.modifiers.new('Clean exterior volume','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.8;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name);modifier=temp.modifiers.new('Relax volume surface','SMOOTH');modifier.factor=.55;modifier.iterations=8;bpy.ops.object.modifier_apply(modifier=modifier.name);cut(temp.data,100,False);cut(temp.data,150,True);middle=temp.data.copy();middle.materials.clear()
 for mat in materials:middle.materials.append(mat)
 for u in list(middle.uv_layers):middle.uv_layers.remove(u)
 for name in uvnames:middle.uv_layers.new(name=name)
 def assign(mesh,polygons):
  for f in polygons:
   center=sum((mesh.vertices[i].co for i in f.vertices),Vector())/len(f.vertices);point,n,index,d=surface.find_nearest(center);slot,uvs=rows[index];f.material_index=slot;tri=[vertices[i]for i in faces[index]]
   for li in f.loop_indices:
    p=mesh.vertices[mesh.loops[li].vertex_index].co;q=closest_point_on_tri(p,*tri)
    for name in uvnames:mesh.uv_layers[name].data[li].uv=barycentric_transform(q,*tri,*uvs[name])[:2]
   f.use_smooth=True
 assign(middle,middle.polygons);low=original.copy();high=original.copy();cut(low,98,True);cut(high,152,False);combined=bmesh.new();combined.from_mesh(low);combined.from_mesh(high);combined.from_mesh(middle);bpy.data.objects.remove(temp,do_unlink=True)
 boundary={e for e in combined.edges if e.is_boundary};loops=[]
 while boundary:
  edge=boundary.pop();ring=[edge.verts[0],edge.verts[1]];current=edge.verts[1]
  while current!=ring[0]:
   candidates=[e for e in current.link_edges if e in boundary]
   assert len(candidates)==1,(len(candidates),list(current.co));edge=candidates[0];boundary.remove(edge);current=edge.other_vert(current)
   if current!=ring[0]:ring.append(current)
  loops.append(ring)
 pairs=[]
 for lowz,highz in [(98,100),(150,152)]:
  aa=[r for r in loops if abs(np.mean([v.co.z for v in r])-lowz)<.01];bb=[r for r in loops if abs(np.mean([v.co.z for v in r])-highz)<.01];assert len(aa)==len(bb),(lowz,len(aa),len(bb))
  for a in aa:
   center=sum((v.co for v in a),Vector())/len(a);b=min(bb,key=lambda r:(sum((v.co for v in r),Vector())/len(r)-center).length);bb.remove(b);pairs.append((a,b))
 old_faces=set(combined.faces)
 for a,b in pairs:
  center=(sum((v.co for v in a),Vector())/len(a)+sum((v.co for v in b),Vector())/len(b))/2
  angle=lambda v:math.atan2(v.co.y-center.y,v.co.x-center.x)%(2*math.pi)
  a=sorted(a,key=angle);b=sorted(b,key=angle);ia=ib=0
  for _ in range(len(a)+len(b)):
   na=angle(a[(ia+1)%len(a)])+(2*math.pi if ia+1>=len(a)else 0);nb=angle(b[(ib+1)%len(b)])+(2*math.pi if ib+1>=len(b)else 0)
   if ia<len(a)and(ib>=len(b)or na<=nb):combined.faces.new((a[ia%len(a)],a[(ia+1)%len(a)],b[ib%len(b)]));ia+=1
   else:combined.faces.new((a[ia%len(a)],b[(ib+1)%len(b)],b[ib%len(b)]));ib+=1
 combined.faces.index_update();newfaceindices=[f.index for f in combined.faces if f not in old_faces];bmesh.ops.recalc_face_normals(combined,faces=list(combined.faces));result=bpy.data.meshes.new('Continuous native fork exterior');combined.to_mesh(result);combined.free()
 for mat in materials:result.materials.append(mat)
 assign(result,[result.polygons[i]for i in newfaceindices]);result.uv_layers.active=result.uv_layers[original.uv_layers.active.name];result.uv_layers[original.uv_layers.active.name].active_render=True;obj.data=result;obj.matrix_world=Matrix.Identity(4);bm=bmesh.new();bm.from_mesh(result);stats=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces),boundary_pairs=len(pairs));bm.free();assert not stats['nonmanifold']and not stats['degenerate'],stats;bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'rebuild.json',dict(model_sha256=sha(out/'model.blend'),source_sha256=sha(source),inference_shape_sha256=sha(trial),topology=stats,scope='Clean own-source local fork exterior atZ100..150, stitched to original rings atZ98/152. Retained source UV and materials outside join. Native coverage and visual review pending.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
