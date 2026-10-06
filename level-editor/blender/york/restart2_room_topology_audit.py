"""Inspect original room-proxy topology before applying physical boolean changes."""
import sys,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,bmesh
p=ROOT/'level-editor/work/york-refinement/restart2/winch-geometry-v2/transition-44/model.blend';bpy.ops.wm.open_mainfile(filepath=str(p));r=[]
for o in bpy.context.scene.objects:
 if o.type!='MESH' or o.get('source_node') not in ['building-764','building-765','building-776','building-779']:continue
 bm=bmesh.new();bm.from_mesh(o.data);r.append({'node':o.get('source_node'),'verts':len(bm.verts),'faces':len(bm.faces),'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),'signed_volume':bm.calc_volume(signed=True)});bm.free()
print(json.dumps(r));(p.parents[1]/'topology-audit.json').write_text(json.dumps(r,indent=2)+'\n')
