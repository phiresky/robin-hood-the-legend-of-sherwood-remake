"""Locate tiny voxel islands before deciding whether they can be discarded."""
import json
import sys
from pathlib import Path
import bpy
import bmesh
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT,SIN,COS
from render_slots import acquire
from evidence_io import sha

acquire()
worker=OUT/'restart2/tree20-v1/assets/croisement01-tree-20'
bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
obj=next(o for o in bpy.data.collections['Croisement01 Working'].all_objects if o.type=='MESH' and o.get('source_node')=='building-082')
bm=bmesh.new();bm.from_mesh(obj.data);remaining=set(bm.verts);reports=[]
while remaining:
    queue=[remaining.pop()];component=[]
    while queue:
        v=queue.pop();component.append(v)
        for edge in v.link_edges:
            other=edge.other_vert(v)
            if other in remaining:remaining.remove(other);queue.append(other)
    if len(component)>16:continue
    points=[obj.matrix_world@v.co for v in component]
    reports.append(dict(vertices=len(component),bounds=[[min(p[i] for p in points),max(p[i] for p in points)] for i in range(3)],native_y=[min(-p.y*SIN-p.z*COS for p in points),max(-p.y*SIN-p.z*COS for p in points)]))
bm.free()
result=dict(model_sha256=sha(worker/'model.blend'),islands=reports)
(worker/'inspection/voxel-fragments.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
