"""Read-only local root and neighboring bank bounds for soil ownership review."""
import json,sys
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement'))
from render_slots import acquire
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';acquire();bpy.ops.wm.open_mainfile(filepath=str(R/'tree01-soil-joint-v8/assets/croisement01-tree-01/model.blend'))
rows=[]
for obj in bpy.data.objects:
 if obj.type!='MESH' or obj.get('source_node') not in {'building-007','building-008','building-029','tree01-local-soil-joint'}:continue
 points=[obj.matrix_world@v.co for v in obj.data.vertices];low=min(p.z for p in points);basal=[p for p in points if p.z<low+20]
 rows.append(dict(node=obj.get('source_node'),bounds=[[min(p[i] for p in points),max(p[i] for p in points)] for i in range(3)],basal_bounds=[[min(p[i] for p in basal),max(p[i] for p in basal)] for i in range(3)],basal_count=len(basal),vertices=len(points),polygons=len(obj.data.polygons)))
(R/'tree01-soil-joint-v8/neighbor-probe.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps(rows,indent=2))
