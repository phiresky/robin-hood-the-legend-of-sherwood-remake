"""Read-only bank material and face provenance inventory before texture scoping."""
import json,sys,collections,hashlib
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';w=R/'tree01-soil-joint-v10/assets/croisement01-tree-01';acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));o=next(x for x in bpy.data.objects if x.type=='MESH' and x.get('source_node')=='building-008');d=dict(model_sha256=hashlib.sha256((w/'model.blend').read_bytes()).hexdigest(),name=o.name,materials=[m.name if m else None for m in o.data.materials],faces_by_material=dict(collections.Counter(p.material_index for p in o.data.polygons)),attributes=[a.name for a in o.data.attributes],uv_layers=[a.name for a in o.data.uv_layers],properties={k:str(v) for k,v in o.items()},bounds=[[min((o.matrix_world@v.co)[i] for v in o.data.vertices),max((o.matrix_world@v.co)[i] for v in o.data.vertices)] for i in range(3)]);(R/'bank008-scope-probe.json').write_text(json.dumps(d,indent=2)+'\n');print(json.dumps(d))
