"""Read-only component and overlap bounds for the native tree24 fork."""
import sys
from pathlib import Path
import bpy,bmesh,numpy as np
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT,SIN,COS
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 source=ROOT/('tree24-fork-union-v4/model.blend' if '--fork' in sys.argv else 'tree24-contour-v2/model.blend');bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();rows=[]
 for obj in bpy.context.scene.objects:
  if obj.type!='MESH'or 'Crown'in obj.name:continue
  bm=bmesh.new();bm.from_mesh(obj.data);remaining=set(bm.verts);components=[]
  while remaining:
   todo=[remaining.pop()];component=set(todo)
   while todo:
    for e in todo.pop().link_edges:
     for v in e.verts:
      if v in remaining:remaining.remove(v);component.add(v);todo.append(v)
   p=np.array([obj.matrix_world@v.co for v in component]);q=np.column_stack((p[:,0],-p[:,1]*SIN-p[:,2]*COS));components.append(dict(vertices=len(component),faces=len({f for v in component for f in v.link_faces}),world_min=p.min(0).tolist(),world_max=p.max(0).tolist(),source_min=q.min(0).tolist(),source_max=q.max(0).tolist()))
  rows.append(dict(object=obj.name,components=components));bm.free()
 write_json(ROOT/('tree24-fork-component-audit-v4.json' if '--fork' in sys.argv else 'tree24-component-audit-v1.json'),dict(source_sha256=sha(source),objects=rows));print(rows,flush=True)
finally:release()
