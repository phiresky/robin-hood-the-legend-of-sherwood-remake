"""Read-only source projections of connected wattle members for correction planning."""
import json,sys
from pathlib import Path
import bpy
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,scenery_workspace
from tree_geometry import SIN,COS
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 asset='croisement02-southwest-path-wattle-fence';worker=scenery_workspace(asset);path=worker/'model.blend';digest=sha(path);bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update();objs=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==asset];records=[]
 for obj in objs:
  vertices=np.array([tuple(obj.matrix_world@v.co)for v in obj.data.vertices]);parent=np.arange(len(vertices))
  def find(i):
   while parent[i]!=i:parent[i]=parent[parent[i]];i=parent[i]
   return i
  for edge in obj.data.edges:
   a,b=map(find,edge.vertices);parent[b]=a
  groups={}
  for i in range(len(vertices)):groups.setdefault(find(i),[]).append(i)
  for indices in groups.values():
   xyz=vertices[indices];source=np.column_stack([xyz[:,0],-SIN*xyz[:,1]-COS*xyz[:,2]])
   records.append(dict(object=obj.name,vertex_indices=indices,vertices=xyz.tolist(),world_min=xyz.min(0).tolist(),world_max=xyz.max(0).tolist(),source_bounds=[*source.min(0).tolist(),*source.max(0).tolist()],likely_post=bool(np.ptp(xyz[:,2])>2*max(np.ptp(xyz[:,0]),np.ptp(xyz[:,1])))))
 dst=OUT/'mixed-wood-audit/boundary-roles76-93-v1/post-geometry.json';write_json(dst,dict(model=str(path),model_sha256=digest,read_only=True,members=records));assert sha(path)==digest;print('Members',len(records),'posts',[r['source_bounds']for r in records if r['likely_post']])

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
