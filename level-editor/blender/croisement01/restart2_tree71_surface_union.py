"""Audit the union of two native-reference surface partitions without editing."""
import sys,json,argparse
from pathlib import Path
import bpy,bmesh
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT
from render_slots import acquire
from evidence_io import sha
parser=argparse.ArgumentParser();parser.add_argument('revision');parser.add_argument('--tree',type=int,choices=[19,71],default=71);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);w=OUT/f'restart2/tree{args.tree}-v{args.revision}/assets/croisement01-tree-{args.tree}';nodes=['building-050','building-051'] if args.tree==19 else ['building-062','building-063']
acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
verts=[];faces=[];parts=[]
for o in bpy.data.objects:
 if o.type!='MESH' or o.get('source_node') not in nodes:continue
 offset=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons);parts.append(o['source_node'])
mesh=bpy.data.meshes.new('Read-only union diagnostic');mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);before=len(bm.verts);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.0001)
record=dict(model_sha256=sha(w/'model.blend'),parts=sorted(parts),input_vertices=before,welded_boundary_duplicates=before-len(bm.verts),vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces),scope='Closed union of touching native-reference surface parts; open per-part seam edges are intentional and neither part adds an internal cap.')
remaining=set(bm.verts);sizes=[]
while remaining:
 pending=[remaining.pop()];count=0
 while pending:
  v=pending.pop();count+=1
  for e in v.link_edges:
   other=e.other_vert(v)
   if other in remaining:remaining.remove(other);pending.append(other)
 sizes.append(count)
record['components']=sorted(sizes,reverse=True);record['status']='PASS' if len(parts)==2 and len(sizes)==1 and record['nonmanifold_edges']==record['degenerate_faces']==0 else 'HOLD'
bm.free();(w/'inspection/saved-wood-surface-union.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record))
