"""Inventory current scoped tree wood before bounded owner-join cleanup."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 out=OUT/'restart2-wood/branch-owner-audit-v1';out.mkdir(exist_ok=False);rows=[]
 for index in [35,43,45,46]:
  worker=tree_workspace(index);bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();parts=[]
  for obj in bpy.data.collections['Croisement02 Working'].all_objects:
   if obj.type!='MESH' or obj.get('asset_group')!=worker.name or obj.get('projection_component')=='crown':continue
   bm=bmesh.new();bm.from_mesh(obj.data);remaining=set(bm.verts);components=[]
   while remaining:
    stack=[remaining.pop()];visited=[]
    while stack:
     v=stack.pop();visited.append(v)
     for e in v.link_edges:
      other=e.other_vert(v)
      if other in remaining:remaining.remove(other);stack.append(other)
    p=np.array([obj.matrix_world@v.co for v in visited]);components.append(dict(vertices=len(visited),min=p.min(axis=0).tolist(),max=p.max(axis=0).tolist()))
   parts.append(dict(object=obj.name,source=obj['source_node'],components=components,faces=len(bm.faces)));bm.free()
  rows.append(dict(mask=index,worker=str(worker),model_sha256=sha(worker/'model.blend'),parts=parts))
 write_json(out/'report.json',rows)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
