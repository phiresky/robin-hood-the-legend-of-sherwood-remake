import bpy,json,sys
from pathlib import Path
from mathutils import Vector
from mathutils.kdtree import KDTree
root=Path.cwd()/'level-editor/work/needs-review/derby-refinement/round-2/recovery-shelters-20260923'
bpy.context.window.scene=bpy.data.scenes['Derby Refinement'];bpy.context.view_layer.update()
report={}
for file in root.glob('*/approved-geometry.json'):
 result={}
 for name,old in json.loads(file.read_text()).items():
  o=bpy.data.objects[name]
  tree=KDTree(len(o.data.vertices))
  for i,v in enumerate(o.data.vertices):tree.insert(o.matrix_world@v.co,i)
  tree.balance()
  result[name]={'same_vertex_count':len(old['vertices'])==len(o.data.vertices),'faces_identical':old['faces']==[list(p.vertices) for p in o.data.polygons],'max_world_vertex_delta':max(((o.matrix_world@v.co)-Vector(p)).length for v,p in zip(o.data.vertices,old['vertices'])),'max_nearest_vertex_delta':max(tree.find(Vector(p))[2] for p in old['vertices'])}
 report[file.parent.name]=result
(root/'live-approved-world-comparison.json').write_text(json.dumps(report,indent=2));print(json.dumps(report))
