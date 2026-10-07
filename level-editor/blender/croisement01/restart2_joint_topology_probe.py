"""Read-only degenerate surface witness for a private terrain joint."""
import json,sys
from pathlib import Path
import bpy,bmesh
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement'))
from render_slots import acquire
from review_evidence import sha
w=Path(sys.argv[sys.argv.index('--')+1]).resolve();out=w/'inspection/joint-topology-detail.json';assert not out.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));rows=[]
for obj in bpy.data.objects:
 if obj.type!='MESH' or obj.get('source_node') not in {'building-003','building-004','building-006','building-007'}:continue
 bm=bmesh.new();bm.from_mesh(obj.data);faces=[dict(area=f.calc_area(),points=[list(obj.matrix_world@v.co) for v in f.verts]) for f in bm.faces if f.calc_area()<1e-8];rows.append(dict(node=obj.get('source_node'),degenerate=faces));bm.free()
out.write_text(json.dumps(dict(model_sha256=sha(w/'model.blend'),objects=rows),indent=2)+'\n');print(out)
