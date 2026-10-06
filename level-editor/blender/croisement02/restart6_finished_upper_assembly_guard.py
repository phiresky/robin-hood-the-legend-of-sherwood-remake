"""Independently bind retained upper UVs and original packed material graph."""
import sys,json,hashlib
from pathlib import Path
import bpy,bmesh
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,SPECS
from render_slots import acquire,release
from evidence_io import sha,write_json
def snapshot(path,tree_id):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();rows=set();cache={}
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or o.get('asset_group')!=f'croisement02-tree-{tree_id}'or'Crown'in o.name:continue
  for face in o.data.polygons:
   if max((o.matrix_world@o.data.vertices[i].co).z for i in face.vertices)<=110:continue
   mat=o.data.materials[face.material_index]
   if mat not in cache:
    nodes=[]
    for n in mat.node_tree.nodes:
     row=dict(name=n.name,type=n.type)
     if n.type=='UVMAP':row['uv_map']=n.uv_map
     if n.type=='TEX_IMAGE':row.update(image_sha256=hashlib.sha256(bytes(n.image.packed_file.data)).hexdigest(),interpolation=n.interpolation,extension=n.extension)
     nodes.append(row)
    graph=dict(nodes=nodes,links=[(l.from_node.name,l.from_socket.name,l.to_node.name,l.to_socket.name)for l in mat.node_tree.links]);cache[mat]=hashlib.sha256(json.dumps(graph,sort_keys=True).encode()).hexdigest()
   uv=o.data.uv_layers['Owned source / exterior']
   for li in face.loop_indices:
    p=o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co
    if p.z>110:rows.add((tuple(round(x,4)for x in p),tuple(round(x,7)for x in uv.data[li].uv),cache[mat]))
 return rows

from restart6_tree18_contour import SPECS as S18
acquire()
try:
 for tree_id,version in [(18,2),(39,1)]:
  source=(S18 if tree_id==18 else SPECS)[tree_id][0];model=ROOT/f'tree{tree_id}-continuous-finished-v{version}/model.blend';a=snapshot(source,tree_id);b=snapshot(model,tree_id);vertices=[];faces=[];lookup={};owners={}
  for o in bpy.context.scene.objects:
   if o.type!='MESH'or o.get('asset_group')!=f'croisement02-tree-{tree_id}'or'Crown'in o.name:continue
   ids=[]
   for v in o.data.vertices:
    key=tuple(o.matrix_world@v.co)
    if key not in lookup:lookup[key]=len(vertices);vertices.append(key)
    ids.append(lookup[key])
   faces.extend([ids[j]for j in f.vertices]for f in o.data.polygons);owners[o.get('source_node')]=dict(name=o.name,faces=len(o.data.polygons),face_owner_values=sorted(set(x.value for x in o.data.attributes['source_building_id'].data))if 'source_building_id'in o.data.attributes else None)
  mesh=bpy.data.meshes.new('Read-only assembled exterior audit');mesh.from_pydata(vertices,[],faces);bm=bmesh.new();bm.from_mesh(mesh);topology=dict(nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-9 for f in bm.faces));bm.free();bpy.data.meshes.remove(mesh)
  receipt=dict(model_sha256=sha(model),approved_source_sha256=sha(source),upper_world_position_uv_material_graph_records_exact=a==b,old_records=len(a),new_records=len(b),missing=len(a-b),extra=len(b-a),assembled_exterior=topology,canonical_owner_objects=owners,scope='Upper loops aboveZ110 preserve position, original UV and packed material graph. Shared exterior part boundaries are joined by exact coordinates for the assembly topology audit; no internal caps.')
  write_json(model.parent/'upper-assembly-guard.json',receipt);print('GUARD',tree_id,receipt,flush=True)
finally:release()
