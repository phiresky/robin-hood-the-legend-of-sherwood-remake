"""Read only selected Tree01/ground receiver metadata for Tree02 context."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def world(o):return world(o.parent)@o.matrix_parent_inverse@o.matrix_basis if o.parent else o.matrix_basis.copy()
def main():
 p=B.parent/'baseline/croisement03-baseline.blend';acquire()
 try:
  bpy.ops.wm.read_factory_settings(use_empty=True)
  with bpy.data.libraries.load(str(p),link=False) as (a,b):
   names=list(a.objects);print('NAMES',names[:25]);b.objects=[n for n in names if n.startswith(('building-002','building-003','building-004','building-005')) or 'ground' in n.lower()]
  rows=[]
  for o in b.objects:
   if o.type!='MESH':continue
   w=world(o);points=[w@v.co for v in o.data.vertices];rows.append(dict(name=o.name,group=o.get('asset_group'),source=o.get('source_node'),vertices=len(points),bounds=[[min(p[i] for p in points) for i in range(3)],[max(p[i] for p in points) for i in range(3)]],world=[list(r) for r in w]))
  out=B/'tree02-context-inventory-v1';out.mkdir(exist_ok=False);write_json(out/'selected-receivers.json',dict(source_model_sha256=sha(p),objects=rows,names=names,scope='Read-only selected receiver survey; no objects or scene saved'));print(json.dumps(rows))
 finally:release()
if __name__=='__main__':main()
