"""Measure the saved split trunk boundary without changing the candidate."""
import sys,json
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT
from render_slots import acquire
acquire();w=OUT/'restart2/tree71-v1/assets/croisement01-tree-71';bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
records=[]
for o in bpy.data.objects:
 if o.type!='MESH' or o.get('source_node') not in ['building-062','building-063']:continue
 points=[o.matrix_world@v.co for v in o.data.vertices];ring=[p for p in points if abs(p.z-150)<.001]
 print(o.name,[(m.name,m.type, getattr(m,'width',None),getattr(m,'factor',None)) for m in o.modifiers]);records.append(dict(node=o['source_node'],plane_ring_vertices=len(ring),bounds=[[min(p[i] for p in ring),max(p[i] for p in ring)] for i in range(3)],mean=[sum(p[i] for p in ring)/len(ring) for i in range(3)]))
(w/'inspection/seam-probe.json').write_text(json.dumps(records,indent=2)+'\n');print(json.dumps(records))
