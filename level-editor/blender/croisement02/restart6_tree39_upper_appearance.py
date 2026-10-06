"""Independently bind retained upper UVs and original packed material graph."""
import sys,json,hashlib
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import ROOT,SPECS
from render_slots import acquire,release
from evidence_io import sha,write_json
def snapshot(path):
 bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();rows=set();cache={}
 for o in bpy.context.scene.objects:
  if o.type!='MESH'or o.get('asset_group')!='croisement02-tree-39'or'Crown'in o.name:continue
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
acquire()
try:
 source=SPECS[39][0];model=ROOT/'tree39-continuous-graft-v4/model.blend';a=snapshot(source);b=snapshot(model);write_json(model.parent/'upper-appearance-guard.json',dict(model_sha256=sha(model),approved_source_sha256=sha(source),upper_world_position_uv_material_graph_records_exact=a==b,old_records=len(a),new_records=len(b),missing=len(a-b),extra=len(b-a),scope='All loops aboveZ110 compare world position, original explicit UV, packed image bytes and node/link graph; material datablock names are immaterial.'));assert a==b,(len(a-b),len(b-a))
finally:release()
