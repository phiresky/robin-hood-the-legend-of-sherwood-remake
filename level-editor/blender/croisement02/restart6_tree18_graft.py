"""Graft the continuous lower source volume onto retained upper wood."""
import sys,json,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree
from mathutils.geometry import barycentric_transform,closest_point_on_tri
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_tree18_contour import ROOT,OUT,SPECS,RAY,SIN,COS
from render_slots import acquire,release
from evidence_io import sha,write_json
from refinement_workspace import _geometry

def cut(mesh,z,lower):
 bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),plane_co=(0,0,z),plane_no=(0,0,1),dist=.00001,clear_outer=lower,clear_inner=not lower);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update()
def main():
 out=ROOT/'tree18-continuous-graft-v1';out.mkdir(exist_ok=False);source,digest=SPECS[18];assert sha(source)==digest;bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-18'];wood=[o for o in objects if 'Crown'not in o.name];crown=next(o for o in objects if 'Crown'in o.name);protected=_geometry(crown,protect_appearance=True);oldparts={o['source_node']:o for o in wood};vertices=[];faces=[];rows=[];upper_original=[]
 for o in wood:
  o.data.calc_loop_triangles();start=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);uv=o.data.uv_layers['Owned source / exterior']
  for t in o.data.loop_triangles:
   ids=tuple(start+j for j in t.vertices);faces.append(ids);rows.append((o.data.materials[t.material_index],[Vector((*uv.data[j].uv,0))for j in t.loops]))
   if min(vertices[j].z for j in ids)>110:upper_original.append(tuple(tuple(vertices[j])for j in ids))
 surface=BVHTree.FromPolygons(vertices,faces,all_triangles=True);item=next(r for r in json.load(open(OUT/'review-mask-inventory.json'))['masks']if r['index']==18);ox,oy=item['box_top_left'];w,h=item['box_size'];rgba=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))[oy:oy+h,ox:ox+w].copy();rgba[:,:,3]=np.array(Image.open(item['png']));Image.fromarray(rgba).save(out/'native18.png');native=bpy.data.images.load(str(out/'native18.png'));native.pack();data=np.load(ROOT/'tree18-native-volume-v2/lower.npz');mesh=bpy.data.meshes.new('Continuous lower tree39');mesh.from_pydata(data['vertices'].tolist(),[],data['faces'].tolist());mesh.update();cut(mesh,.1501,False);cut(mesh,104,True);lower=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(lower);uv=mesh.uv_layers.new(name='Owned source / exterior');native_uv=mesh.uv_layers.new(name='Continuous lower native projection');slots={};source_near=0;new_neutral=0
 for f in mesh.polygons:
  p,n,i,d=surface.find_nearest(f.center);base,old_uv=rows[i];native_front=f.normal.dot(RAY)>0;near=d<=3;key=(base.name,native_front,near)
  if key not in slots:
   mat=base.copy();mat.name=base.name+' / continuous lower '+('native'if native_front else'rear')+(' retained'if near else' inferred');nodes=mat.node_tree.nodes;output=next(n for n in nodes if n.type=='OUTPUT_MATERIAL');socket=output.inputs['Surface'];old=socket.links[0].from_socket
   if not near:
    value=nodes.new('ShaderNodeRGB');value.outputs[0].default_value=(.18,.18,.18,1);old=value.outputs[0]
   if native_front:
    tex=nodes.new('ShaderNodeTexImage');tex.image=native;tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=native_uv.name;mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(uvnode.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket)
   elif not near:mat.node_tree.links.new(old,socket)
   mesh.materials.append(mat);slots[key]=len(mesh.materials)-1
  f.material_index=slots[key];source_near+=int(near);new_neutral+=int(not near);tri=[vertices[j]for j in faces[i]]
  for li in f.loop_indices:
   v=mesh.vertices[mesh.loops[li].vertex_index].co;q=closest_point_on_tri(v,*tri);t=barycentric_transform(q,*tri,*old_uv);uv.data[li].uv=t[:2];native_uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
  f.use_smooth=True
 original=oldparts['building-101'];uppermesh=original.data.copy();uppermesh.transform(original.matrix_world);cut(uppermesh,100,False);upper=bpy.data.objects.new('Exact original upper wood',uppermesh);scene.collection.objects.link(upper);old_slots=list(uppermesh.materials);old_indices=[f.material_index for f in uppermesh.polygons];offset=len(lower.data.materials)
 for material in old_slots:lower.data.materials.append(material)
 uppermesh.materials.clear()
 for material in lower.data.materials:uppermesh.materials.append(material)
 for face,index in zip(uppermesh.polygons,old_indices):face.material_index=index+offset
 bpy.ops.object.select_all(action='DESELECT');lower.select_set(True);bpy.context.view_layer.objects.active=lower
 modifier=lower.modifiers.new('Continuous lower to exact upper union','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.use_self=False;modifier.object=upper;bpy.ops.object.modifier_apply(modifier=modifier.name);combined=lower.data.copy();bpy.data.objects.remove(lower,do_unlink=True);bpy.data.objects.remove(upper,do_unlink=True);combined.calc_loop_triangles();whole=BVHTree.FromPolygons([v.co for v in combined.vertices],[list(p.vertices)for p in combined.polygons]);upper_dist=max(whole.find_nearest(Vector(p))[3]for tri in upper_original for p in tri);assert upper_dist<.002,upper_dist;normals=KDTree(len(combined.vertices));normal_values=[]
 for v in combined.vertices:normals.insert(v.co,v.index);normal_values.append(tuple(v.normal))
 normals.balance();checks={}
 for node,o in oldparts.items():
  result=combined.copy();bm=bmesh.new();bm.from_mesh(result)
  for _ in range(100):
   tiny=[f for f in bm.faces if f.calc_area()<1e-9 and max(v.co.z for v in f.verts)<110]
   if not tiny:break
   edge=min(tiny[0].edges,key=lambda e:e.calc_length());assert edge.calc_length()<.01;bmesh.ops.collapse(bm,edges=[edge],uvs=False)
  bm.to_mesh(result);bm.free();result.update();o.parent=None;o.matrix_world=Matrix.Identity(4);o.data=result
  for p in result.polygons:p.use_smooth=True
  result.normals_split_custom_set_from_vertices([normal_values[normals.find(v.co)[1]]for v in result.vertices]);bm=bmesh.new();bm.from_mesh(result);checks[node]=dict(vertices=len(result.vertices),faces=len(result.polygons),boundary_edges=sum(e.is_boundary for e in bm.edges),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free()
  uv=result.uv_layers['Continuous lower native projection']
  for li,loop in enumerate(result.loops):
   v=result.vertices[loop.vertex_index].co;uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
 # Confirmed lower native hits receive their exact observed RGB, including
 # grazing toe faces that changed orientation during the silhouette fit.
 from refinement_review import _tree
 bpy.context.view_layer.update();tree,owners,_=_tree(wood);tris=[]
 for o in wood:o.data.calc_loop_triangles();tris.extend((o,t)for t in o.data.loop_triangles)
 observed=np.array(Image.open(item['png']))>0;yy,xx=np.where(observed);fixes={}
 for sy,sx in zip(yy+oy,xx+ox):
  hit,n,i,d=tree.ray_cast(Vector((sx+.5,-(sy+.5)/SIN,0))+RAY*6000,-RAY)
  if hit is None or hit.z>104:continue
  o,t=tris[i];face=o.data.polygons[t.polygon_index];base=o.data.materials[face.material_index]
  if any(n.type=='TEX_IMAGE'and n.image==native for n in base.node_tree.nodes):continue
  key=(o.name,base.name)
  if key not in fixes:
   mat=base.copy();mat.name+=' / observed lower source';nodes=mat.node_tree.nodes;socket=next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket;tex=nodes.new('ShaderNodeTexImage');tex.image=native;tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map='Continuous lower native projection';mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(uvnode.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket);o.data.materials.append(mat);fixes[key]=len(o.data.materials)-1
  face.material_index=fixes[key]
 assert _geometry(crown,protect_appearance=True)==protected
 for o in list(bpy.data.objects):
  if o.type=='MESH'and o not in objects:bpy.data.objects.remove(o,do_unlink=True)
 for o in objects:o.hide_render=False
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'construction.json',dict(model_sha256=sha(out/'model.blend'),approved_parent_sha256=digest,lower_volume_sha256=sha(ROOT/'tree18-native-volume-v2/lower.npz'),crown_exact=True,upper_surface_distance_max=upper_dist,parts=checks,retained_near_surface_faces=source_near,new_neutral_faces=new_neutral,scope='Continuous lower wood; exact upper support surface/crown retained. Single continuous wood owner retains its original source identity. Observed lower native RGB overlays retained near-surface atlas reuse; new unsupported surfaces gray, not synthesized.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
