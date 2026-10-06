"""Private continuous lower toe hypotheses; retain original upper wood and crown."""
import sys,json,math,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform,closest_point_on_tri
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,OUT,RAY,SIN,COS
from restart6_tree24_fork_rebuild import cut
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry

def rings(bm):
 boundary={e for e in bm.edges if e.is_boundary};result=[]
 while boundary:
  e=boundary.pop();r=[e.verts[0],e.verts[1]];v=e.verts[1]
  while v!=r[0]:
   edges=[e for e in v.link_edges if e in boundary];assert len(edges)==1,(len(edges),list(v.co));e=edges[0];boundary.remove(e);v=e.other_vert(v)
   if v!=r[0]:r.append(v)
  result.append(r)
 return result

def main(number):
 assert shutil.disk_usage(OUT).free>25*2**30
 prior=json.load(open(ROOT/f'baseline-audit-{number}-v3/report.json'));source=Path(prior['source']);assert sha(source)==prior['source_sha256'];out=ROOT/f'tree{number}-toe-union-v10';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();scene=bpy.context.scene;asset=f'croisement02-tree-{number}';own=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];wood=[o for o in own if o.get('projection_component')!='crown'];crowns={o.name:_geometry(o,protect_appearance=True)for o in own if o not in wood};metadata={o['source_node']:dict(name=o.name,properties={k:o[k]for k in o.keys()})for o in wood};vertices=[];faces=[];rows=[];materials=[];uvnames=sorted({u.name for o in wood for u in o.data.uv_layers});active=wood[0].data.uv_layers.active.name
 for o in wood:
  o.data.calc_loop_triangles();offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices)
  for t in o.data.loop_triangles:
   material=o.data.materials[t.material_index]
   if material not in materials:materials.append(material)
   faces.append(tuple(offset+i for i in t.vertices));rows.append((materials.index(material),{name:[Vector((*o.data.uv_layers[name].data[li].uv,0))for li in t.loops]if name in o.data.uv_layers else[Vector((0,0,0))]*3 for name in uvnames},int(o['source_node'].split('-')[-1])))
 surface=BVHTree.FromPolygons(vertices,faces,all_triangles=True);original=bpy.data.meshes.new('Original world wood');original.from_pydata(vertices,[],faces);original.update()
 for m in materials:original.materials.append(m)
 for name in uvnames:original.uv_layers.new(name=name)
 owner=original.attributes.new('source_building_id','INT','FACE')
 for face,(slot,uvs,node)in zip(original.polygons,rows):
  face.material_index=slot;owner.data[face.index].value=node
  for li,j in zip(face.loop_indices,range(3)):
   for name in uvnames:original.uv_layers[name].data[li].uv=uvs[name][j][:2]
 limit=42 if number==19 else 70;data=np.load(ROOT/f'tree{number}-toe-volume-v2/lower.npz');inferred=bpy.data.meshes.new('True lower source volume');inferred.from_pydata(data['vertices'].tolist(),[],data['faces'].tolist());inferred.update();temp=bpy.data.objects.new('Private true lower union',inferred);scene.collection.objects.link(temp)
 # Ground clip produces a flat physical contact, not a downward floating tip.
 cut(temp.data,.15,False);bm=bmesh.new();bm.from_mesh(temp.data);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(temp.data);bm.free();cut(temp.data,limit-1,True);low=temp.data.copy();low.materials.clear()
 for m in materials:low.materials.append(m)
 for u in list(low.uv_layers):low.uv_layers.remove(u)
 for name in uvnames:low.uv_layers.new(name=name)
 if 'source_building_id'not in low.attributes:low.attributes.new('source_building_id','INT','FACE')
 def assign(mesh,fs):
  for f in fs:
   center=sum((mesh.vertices[i].co for i in f.vertices),Vector())/len(f.vertices);p,n,index,d=surface.find_nearest(center);slot,uvs,node=rows[index];f.material_index=slot;mesh.attributes['source_building_id'].data[f.index].value=node;tri=[vertices[i]for i in faces[index]]
   for li in f.loop_indices:
    v=mesh.vertices[mesh.loops[li].vertex_index].co;q=closest_point_on_tri(v,*tri)
    for name in uvnames:mesh.uv_layers[name].data[li].uv=barycentric_transform(q,*tri,*uvs[name])[:2]
   f.use_smooth=True
 assign(low,low.polygons);high=original.copy();cut(high,limit+1,False);bm=bmesh.new();bm.from_mesh(high);bm.from_mesh(low);rr=rings(bm);aa=[r for r in rr if abs(np.mean([v.co.z for v in r])-(limit-1))<.01];bb=[r for r in rr if abs(np.mean([v.co.z for v in r])-(limit+1))<.01];assert len(aa)==len(bb),(len(aa),len(bb));oldfaces=set(bm.faces)
 for a in aa:
  ca=sum((v.co for v in a),Vector())/len(a);b=min(bb,key=lambda r:(sum((v.co for v in r),Vector())/len(r)-ca).length);bb.remove(b);center=(ca+sum((v.co for v in b),Vector())/len(b))/2;angle=lambda v:math.atan2(v.co.y-center.y,v.co.x-center.x)%(2*math.pi);a=sorted(a,key=angle);b=sorted(b,key=angle);ia=ib=0
  for _ in range(len(a)+len(b)):
   na=angle(a[(ia+1)%len(a)])+(2*math.pi if ia+1>=len(a)else 0);nb=angle(b[(ib+1)%len(b)])+(2*math.pi if ib+1>=len(b)else 0)
   if ia<len(a)and(ib>=len(b)or na<=nb):bm.faces.new((a[ia%len(a)],a[(ia+1)%len(a)],b[ib%len(b)]));ia+=1
   else:bm.faces.new((a[ia%len(a)],b[(ib+1)%len(b)],b[ib%len(b)]));ib+=1
 bm.faces.index_update();newids=[f.index for f in bm.faces if f not in oldfaces];bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));mesh=bpy.data.meshes.new('Continuous grounded toe');bm.to_mesh(mesh);bm.free()
 for m in materials:mesh.materials.append(m)
 assign(mesh,[mesh.polygons[i]for i in newids]);mesh.uv_layers.active=mesh.uv_layers[active];mesh.uv_layers[active].active_render=True
 # Source-visible lower surfaces retain native source colors, including observed ivy.
 item=next(x for x in json.load(open(OUT/'review-mask-inventory.json'))['masks']if x['index']==number);ox,oy=item['box_top_left'];w,h=item['box_size'];rgba=np.array(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'))[oy:oy+h,ox:ox+w].copy();rgba[:,:,3]=np.array(Image.open(item['png']).convert('L'));Image.fromarray(rgba).save(out/f'native{number}.png');image=bpy.data.images.load(str(out/f'native{number}.png'));image.pack();uv=mesh.uv_layers.new(name='Continuous toe native projection');mesh.uv_layers.active=mesh.uv_layers[active];mesh.uv_layers[active].active_render=True;slots={}
 for li,loop in enumerate(mesh.loops):
  v=mesh.vertices[loop.vertex_index].co;uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
 for f in mesh.polygons:
  f.use_smooth=True
  if min(mesh.vertices[i].co.z for i in f.vertices)>limit+1.01:continue
  slot=f.material_index;native=f.normal.dot(RAY)>.02;distance=surface.find_nearest(f.center)[3];unknown=distance>2;key=(slot,native,unknown)
  if key not in slots:
   mat=materials[slot].copy();mat.name+=' / private continuous toe '+('native'if native else'inferred reverse');nodes=mat.node_tree.nodes;socket=next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket
   if unknown:
    value=nodes.new('ShaderNodeRGB');value.outputs[0].default_value=(.18,.18,.18,1);old=value.outputs[0];mat.node_tree.links.new(old,socket)
   if native:
    tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';coord=nodes.new('ShaderNodeUVMap');coord.uv_map=uv.name;mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(coord.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket)
   mesh.materials.append(mat);slots[key]=len(mesh.materials)-1
  f.material_index=slots[key]
 bm=bmesh.new();bm.from_mesh(mesh)
 for _ in range(20):
  tiny=[f for f in bm.faces if f.calc_area()<1e-9]
  if not tiny:break
  edge=min(tiny[0].edges,key=lambda e:e.calc_length());assert edge.calc_length()<.001;bmesh.ops.collapse(bm,edges=[edge],uvs=True)
 bm.to_mesh(mesh);mesh.update();stats=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free();assert stats['nonmanifold']==0 and stats['degenerate']==0,stats
 for o in wood:bpy.data.objects.remove(o,do_unlink=True)
 bpy.data.objects.remove(temp,do_unlink=True)
 for node,meta in metadata.items():
  partmesh=mesh.copy();bm=bmesh.new();bm.from_mesh(partmesh);layer=bm.faces.layers.int['source_building_id'];bmesh.ops.delete(bm,geom=[f for f in bm.faces if f[layer]!=int(node.split('-')[-1])],context='FACES');bm.to_mesh(partmesh);bm.free();o=bpy.data.objects.new(meta['name'],partmesh);scene.collection.objects.link(o)
  for k,v in meta['properties'].items():o[k]=v
  o['continuous_exterior_group']=asset;o['internal_caps']=False
 for o in list(bpy.data.objects):
  if o.type=='MESH'and o.get('asset_group')!=asset:bpy.data.objects.remove(o,do_unlink=True)
 for o in scene.objects:
  if o.name in crowns:assert _geometry(o,protect_appearance=True)==crowns[o.name]
  if o.type=='MESH':o.hide_render=False
 bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'construction.json',dict(model_sha256=sha(out/'model.blend'),source=str(source),source_sha256=sha(source),crown_exact=True,upper_preservation_z=limit+1,assembled_topology=stats,source_parts=list(metadata),scope='Private continuous grounded lower toe hypothesis. Original upper wood/crown retained; native lower RGB protected. Exact coverage and contact review pending. No claim every dark fringe is physical wood.'))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
