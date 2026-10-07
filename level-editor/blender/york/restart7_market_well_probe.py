"""Read-only market well geometry and source authority inventory."""
import sys,json,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
B=ROOT/'level-editor/work/york-refinement';D=B/'restart7-market-well-study-v1';MODEL=B/'grounding/york-grounded.blend'
def main():
 bpy.ops.wm.open_mainfile(filepath=str(MODEL));bpy.context.view_layer.update();rows=[]
 for o in bpy.data.collections['york Working'].all_objects:
  if o.type!='MESH' or o.get('source_node')not in [f'building-{i:03d}'for i in [35,36,37,86]]:continue
  rows.append(dict(name=o.name,properties={k:o[k]for k in o.keys()},matrix=[list(r)for r in o.matrix_world],vertices=[list(o.matrix_world@v.co)for v in o.data.vertices],faces=[list(p.vertices)for p in o.data.polygons],materials=[m.name for m in o.data.materials if m]))
 d=dict(source=str(MODEL),sha256=hashlib.sha256(MODEL.read_bytes()).hexdigest(),objects=rows);(D/'geometry-probe.json').write_text(json.dumps(d,indent=2)+'\n');print([(r['name'],len(r['vertices']),len(r['faces']))for r in rows])
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
