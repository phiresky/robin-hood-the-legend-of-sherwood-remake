"""Inspect component ownership and projected support without changing receivers."""
import sys,json
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart6_source_gap_audit import SPECS,ROOT,SIN,COS
from evidence_io import write_json
from render_slots import acquire,release
def main(index):
 bpy.ops.wm.open_mainfile(filepath=str(SPECS[index][0]));bpy.context.view_layer.update();asset='croisement02-tree-39'if index==39 else'croisement02-east-upright-rail-fence-95';rows=[]
 for obj in bpy.context.scene.objects:
  if obj.type!='MESH'or obj.get('asset_group')!=asset or 'Crown'in obj.name:continue
  points=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);projected=np.column_stack((points[:,0],-points[:,1]*SIN-points[:,2]*COS));parent=list(range(len(points)))
  def find(i):
   while parent[i]!=i:parent[i]=parent[parent[i]];i=parent[i]
   return i
  for e in obj.data.edges:
   a,b=e.vertices;parent[find(a)]=find(b)
  components={}
  for i in range(len(points)):components.setdefault(find(i),[]).append(i)
  for k,ids in components.items():
   p=points[ids];q=projected[ids];rows.append(dict(object=obj.name,component=k,vertices=ids,world_vertices=p.tolist(),source_vertices=q.tolist(),world_bounds=[p.min(0).tolist(),p.max(0).tolist()],source_bounds=[q.min(0).tolist(),q.max(0).tolist()]))
 write_json(ROOT/f'components-{index}-v2.json',dict(components=rows))
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
