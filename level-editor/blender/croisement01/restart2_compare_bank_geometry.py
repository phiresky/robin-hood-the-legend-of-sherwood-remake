"""Check whether refreshed support constraints changed any saved world geometry."""
import hashlib,json,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/active/croisement01/restart2';acquire(slots=2)
try:
 rows=[]
 for revision in [14,15]:
  model=R/f'tree05-regional-banks-v{revision}/assets/croisement01-tree-05/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));objects={}
  for o in bpy.context.scene.objects:
   if o.type!='MESH':continue
   h=hashlib.sha256();v=np.empty(len(o.data.vertices)*3,dtype=np.float32);o.data.vertices.foreach_get('co',v);h.update(v.tobytes());h.update(np.asarray(o.matrix_world,dtype=np.float64).tobytes());loops=np.empty(len(o.data.loops),dtype=np.int32);o.data.loops.foreach_get('vertex_index',loops);h.update(loops.tobytes());counts=np.empty(len(o.data.polygons),dtype=np.int32);o.data.polygons.foreach_get('loop_total',counts);h.update(counts.tobytes());objects[o.name]=h.hexdigest()
  rows.append(dict(revision=revision,model_sha256=sha(model),mesh_geometry=objects))
 changed=[key for key in rows[0]['mesh_geometry'].keys()|rows[1]['mesh_geometry'].keys() if rows[0]['mesh_geometry'].get(key)!=rows[1]['mesh_geometry'].get(key)];p=R/'tree05-regional-banks-v15/geometry-comparison-v14.json';p.write_text(json.dumps(dict(status='EXACT_GEOMETRY' if not changed else 'GEOMETRY_CHANGED',sources=rows,changed=changed),indent=2)+'\n');print('changed',changed)
finally:release()
