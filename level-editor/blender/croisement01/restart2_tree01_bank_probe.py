"""Read-only topology witness for the archived bank and private shoulder."""
import json,sys
from pathlib import Path
import bpy,bmesh
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT
from render_slots import acquire
acquire();result=[]
for revision in ['tree01-v2']:
 path=OUT/'restart2'/revision/'assets/croisement01-tree-01/model.blend';bpy.ops.wm.open_mainfile(filepath=str(path))
 o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='building-008');bm=bmesh.new();bm.from_mesh(o.data);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.01);bm.verts.ensure_lookup_table();bm.verts.index_update();result.append(dict(points=[list(v.co) for v in bm.verts],polygons=[[v.index for v in f.verts] for f in bm.faces],revision=revision,vertices=len(bm.verts),faces=len(bm.faces),boundary_edges=sum(e.is_boundary for e in bm.edges),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),volume=bm.calc_volume(signed=True),bounds=[[min(v.co[i] for v in bm.verts),max(v.co[i] for v in bm.verts)] for i in range(3)]));bm.free()
print(json.dumps(result));(OUT/'restart2/tree01-bank-topology-probe.json').write_text(json.dumps(result,indent=2)+'\n')
