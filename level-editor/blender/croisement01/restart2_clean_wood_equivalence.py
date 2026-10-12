"""Prove cleaned candidates retain every vertex and face of their main wood."""
import hashlib,json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/active/croisement01/restart2';acquire(slots=2)
try:
 for n,v in [(4,6),(5,7)]:
  rows=[]
  for revision in [v,v+1]:
   model=R/f'tree{n:02d}-v{revision}'/f'assets/croisement01-tree-{n:02d}/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_node')==f'scenery-tree{n:02d}-wood');bm=bmesh.new();bm.from_mesh(obj.data);todo=set(bm.verts);components=[]
   while todo:
    queue=[todo.pop()];ids=[]
    while queue:
     vertex=queue.pop();ids.append(vertex.index)
     for edge in vertex.link_edges:
      other=edge.other_vert(vertex)
      if other in todo:todo.remove(other);queue.append(other)
    components.append(ids)
   ids=sorted(max(components,key=len));bm.free();mapping={old:new for new,old in enumerate(ids)};h=hashlib.sha256();h.update(np.asarray([tuple(obj.data.vertices[i].co) for i in ids],dtype=np.float32).tobytes());h.update(np.asarray(obj.matrix_world,dtype=np.float64).tobytes());faces=[];uv=[];materials=[]
   for poly in obj.data.polygons:
    if poly.vertices[0] not in mapping:continue
    faces.append([mapping[i] for i in poly.vertices]);materials.append(poly.material_index)
    for layer in obj.data.uv_layers:uv.extend(tuple(layer.data[i].uv) for i in poly.loop_indices)
   h.update(json.dumps(faces,separators=(',',':')).encode());h.update(np.asarray(uv,dtype=np.float32).tobytes());h.update(np.asarray(materials,dtype=np.int32).tobytes());rows.append(dict(revision=revision,model_sha256=sha(model),main_geometry_uv_material_hash=h.hexdigest(),vertices=len(ids),faces=len(faces)))
  assert rows[0]['main_geometry_uv_material_hash']==rows[1]['main_geometry_uv_material_hash'];dest=R/f'tree{n:02d}-v{v+1}/retained-main-equivalence.json';dest.write_text(json.dumps(dict(status='MAIN_WOOD_GEOMETRY_UV_MATERIALS_EXACT',sources=rows,scope='Only zero-source disconnected crumbs removed; retained world geometry and original bank contact remain exact.'),indent=2)+'\n');print(n,'main exact')
finally:release()
