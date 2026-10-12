import sys,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,bmesh
bpy.ops.wm.open_mainfile(filepath=str(ROOT/'level-editor/work/active/york/restart2/restart42-channel-assembly-v1/model.blend'))
for o in bpy.context.scene.objects:
 if o.type=='MESH' and o.get('source_node')in {'building-764','building-765','building-776','building-779'}:
  b=bmesh.new();b.from_mesh(o.data);bmesh.ops.remove_doubles(b,verts=list(b.verts),dist=.0001);bmesh.ops.recalc_face_normals(b,faces=list(b.faces));print(json.dumps({'name':o.name,'verts':len(b.verts),'faces':len(b.faces),'boundary':sum(e.is_boundary for e in b.edges),'nonmanifold':sum(not e.is_manifold for e in b.edges),'volume':b.calc_volume(signed=True),'determinant':o.matrix_world.determinant()}));b.free()
