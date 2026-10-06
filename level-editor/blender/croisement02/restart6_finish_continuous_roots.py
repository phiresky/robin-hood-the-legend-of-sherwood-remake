"""Finish bounded root transitions and retain uncapped canonical exterior parts."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector,Matrix
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree18_contour import SPECS as S18
from restart6_tree39_contour import SPECS as S39,ROOT,OUT,RAY,SIN,COS,covered
from restart6_graft_transition import blend
from refinement_review import _tree
from refinement_workspace import _geometry
from render_slots import acquire,release
from evidence_io import sha,write_json
from PIL import Image

def main(tree_id):
 spec=(S18 if tree_id==18 else S39)[tree_id];asset=f'croisement02-tree-{tree_id}';parent=ROOT/('tree18-continuous-graft-v2'if tree_id==18 else'tree39-exterior-rebuild-v2')/'model.blend';out=ROOT/f'tree{tree_id}-continuous-finished-v1';out.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(spec[0]));meta={o['source_node']:dict(name=o.name,properties={k:o[k]for k in o.keys()})for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset and'Crown'not in o.name}
 bpy.ops.wm.open_mainfile(filepath=str(parent));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset and'Crown'not in o.name);crown=next(o for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset and'Crown'in o.name);protected=_geometry(crown,protect_appearance=True);transition=blend(obj);assert transition['nonmanifold']==0 and transition['degenerate']==0,transition
 inv=json.load(open(OUT/'review-mask-inventory.json'));item=next(x for x in inv['masks']if x['index']==tree_id);ox,oy=item['box_top_left'];w,h=item['box_size'];uv=obj.data.uv_layers['Continuous lower native projection']
 for li,loop in enumerate(obj.data.loops):
  v=obj.data.vertices[loop.vertex_index].co;uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
 native=next(im for im in bpy.data.images if im.name.startswith(f'native{tree_id}'));yy,xx=np.where(np.array(Image.open(item['png']))>0);bpy.context.view_layer.update();bvh,_,_=_tree([obj]);obj.data.calc_loop_triangles();triangles=list(obj.data.loop_triangles);fixes={}
 for y,x in zip(yy+oy,xx+ox):
  hit,n,i,d=bvh.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
  if hit is None or hit.z>=110:continue
  face=obj.data.polygons[triangles[i].polygon_index];base=obj.data.materials[face.material_index]
  if any(n.type=='TEX_IMAGE'and n.image==native for n in base.node_tree.nodes):continue
  if base.name not in fixes:
   mat=base.copy();mat.name+=' / exact observed native';nodes=mat.node_tree.nodes;socket=next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];old=socket.links[0].from_socket;tex=nodes.new('ShaderNodeTexImage');tex.image=native;tex.interpolation='Closest';tex.extension='CLIP';uvnode=nodes.new('ShaderNodeUVMap');uvnode.uv_map=uv.name;mix=nodes.new('ShaderNodeMixRGB');mat.node_tree.links.new(uvnode.outputs['UV'],tex.inputs['Vector']);mat.node_tree.links.new(tex.outputs['Alpha'],mix.inputs[0]);mat.node_tree.links.new(old,mix.inputs[1]);mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[2]);mat.node_tree.links.new(mix.outputs[0],socket);obj.data.materials.append(mat);fixes[base.name]=len(obj.data.materials)-1
  face.material_index=fixes[base.name]
 owner_rows={}
 if tree_id==39:
  full=obj.data;normals=[tuple(v.normal)for v in full.vertices];face_owner=[95 if f.center.z>=70 else(96 if f.center.x<1705 else 97)for f in full.polygons];attribute=full.attributes.new('source_building_id','INT','FACE')
  for entry,node in zip(attribute.data,face_owner):entry.value=node
  # Split existing exterior faces only. No cap is created at ownership boundaries.
  for node in [95,96,97]:
   mesh=full.copy();bm=bmesh.new();bm.from_mesh(mesh);layer=bm.faces.layers.int['source_building_id'];bmesh.ops.delete(bm,geom=[f for f in bm.faces if f[layer]!=node],context='FACES');bm.to_mesh(mesh);bm.free();mesh.update();key=f'building-{node:03d}';part=bpy.data.objects.new(meta[key]['name']+' temporary',mesh);scene.collection.objects.link(part)
   for k,v in meta[key]['properties'].items():part[k]=v
   part['source_nodes']=[key];part['continuous_exterior_group']=asset;part['internal_caps']=False;part.matrix_world=Matrix.Identity(4)
   # Match normals at identical shared coordinates without changing geometry.
   lookup={tuple(v.co):normals[v.index]for v in full.vertices};mesh.normals_split_custom_set_from_vertices([lookup[tuple(v.co)]for v in mesh.vertices]);owner_rows[key]=dict(object=meta[key]['name'],faces=len(mesh.polygons),shared_boundary_is_uncapped=True)
  bpy.data.objects.remove(obj,do_unlink=True)
  for o in scene.objects:
   if o.type=='MESH'and o.get('asset_group')==asset and'Crown'not in o.name:o.name=meta[o['source_node']]['name']
 assert _geometry(crown,protect_appearance=True)==protected;bpy.ops.outliner.orphans_purge(do_recursive=True);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'construction.json',dict(model_sha256=sha(out/'model.blend'),parent_sha256=sha(parent),approved_parent_sha256=spec[1],transition=transition,canonical_owners=owner_rows,crown_exact=True,scope='Continuous exterior, rounded local graft below Z110. Canonical owner parts share exterior boundaries without artificial caps; assembled surface is closed. Exact native projection overlay retained. New reverse surfaces remain pending appearance.'))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
