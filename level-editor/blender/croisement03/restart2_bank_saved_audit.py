"""Reopen the bounded bank and audit topology, metadata and unchanged path geometry."""
import sys,json,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement')]
from restart2_bank_full_v1 import world
from render_slots import acquire,release
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement';O=Path(sys.argv[sys.argv.index('--')+1]).resolve() if '--' in sys.argv else B/'restart2/bank-full-prototype-v2'
def fingerprint(o):
 m=o.data
 return dict(vertices=[list(v.co) for v in m.vertices],faces=[list(f.vertices) for f in m.polygons],uvs=[list(v.uv) for v in m.uv_layers.active.data],world=[list(r) for r in world(o)])
def main():
 assert not (O/'saved-audit-v2.json').exists();acquire()
 try:
  model=O/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene;level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());topology={};context={}
  for o in s.objects:
   if o.type!='MESH':continue
   if o.name.startswith('Candidate bank'):
    assert json.loads(o['gameplay_metadata_json'])==level['sight_obstacles'][o['native_obstacle']]
    bm=bmesh.new();bm.from_mesh(o.data);topology[o.name]=dict(vertices=len(bm.verts),faces=len(bm.faces),materials=[m.name if m else None for m in o.data.materials],material_indices=sorted(set(f.material_index for f in o.data.polygons)),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),boundary_edges=sum(e.is_boundary for e in bm.edges),zero_area_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free()
   else:context[o.get('source_node',o.name)]=fingerprint(o)
  if (O/'interface-construction.json').exists():
   from mathutils.bvhtree import BVHTree
   from mathutils import Vector
   receipt=json.loads((O/'interface-construction.json').read_text());plan_path=Path(receipt.get('geometry_plan',B/'restart2/bank-whole-geometry-plan-v3/geometry.json'));plan=json.loads(plan_path.read_text());trace_errors={};contact_errors={}
   for index in ('52','54'):
    o=s.objects['Candidate bank '+index];m=o.data;m.calc_loop_triangles();tree=BVHTree.FromPolygons([v.co for v in m.vertices],[list(t.vertices) for t in m.loop_triangles],all_triangles=True)
    errors=[]
    for trace in plan[index]['traces']:
     for i in trace['vertices']:errors.append(tree.find_nearest(Vector(plan[index]['vertices'][i]))[3])
    trace_errors[index]=max(errors)
    S=__import__('math').sin(__import__('math').radians(35));C=__import__('math').cos(__import__('math').radians(35))
    contact_errors[index]=max(tree.find_nearest(Vector((x,-y/S,z/C)))[3] for x,y,z in plan['guards']['interface52_54'])
   write_json(O/'saved-trace-interface-guard.json',dict(model_sha256=digest,trace_max_world_error=trace_errors,shared52_54_endpoint_error=contact_errors,status='PASS' if max([*trace_errors.values(),*contact_errors.values()])<1e-4 else 'HOLD trace/interface'))
  base=B/'baseline/croisement03-baseline.blend'
  with bpy.data.libraries.load(str(base),link=False) as (a,b):b.objects=[f'building-{i:03}.001' for i in (94,95,96,97)]
  differences=[];world_errors={}
  for o in b.objects:
   key=o.get('source_node',o.name);before=fingerprint(o);after=context[key]
   for part in before:
    if part=='world':
     error=float(np.max(np.abs(np.array(before[part])-np.array(after[part]))));world_errors[key]=error
     if error>1e-4:differences.append([key,part,error])
    elif before[part]!=after[part]:differences.append([key,part])
  assert sha(model)==digest;write_json(O/'saved-audit-v2.json',dict(model_sha256=digest,bank_topology=topology,unchanged_context_differences=differences,context_world_max_error=world_errors,context_transform_tolerance=1e-4,gameplay_metadata_exact=True,status='PASS topology/context' if not differences and all(not r['nonmanifold_edges'] and not r['zero_area_faces'] for r in topology.values()) else 'HOLD topology or context differences'))
 finally:release()
if __name__=='__main__':main()
