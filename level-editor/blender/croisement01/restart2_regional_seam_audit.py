"""Inspect world-welded regional boundaries without changing saved terrain."""
import argparse
import json
import sys
from pathlib import Path
import bpy
import bmesh

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha

p=argparse.ArgumentParser();p.add_argument('worker',type=Path)
a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.worker.resolve()
acquire(slots=2);bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'))
nodes=set(json.loads((w.parents[1]/'construction.json').read_text())['changed_nodes'])
objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_node') in nodes]
assert len(objects)==len(nodes)
points=[];faces=[];upward_audit=[]
for o in objects:
    offset=len(points);points.extend(o.matrix_world@v.co for v in o.data.vertices)
    faces.extend(tuple(offset+i for i in f.vertices) for f in o.data.polygons)
    upper=[f for f in o.data.polygons if all((o.matrix_world@o.data.vertices[k].co).z>-.999 for k in f.vertices)]
    downward=[f.index for f in upper if (o.matrix_world.to_3x3().inverted().transposed()@f.normal).z<-.1]
    upward_audit.append(dict(node=o.get('source_node'),upper_faces=len(upper),downward_upper_faces=downward))
mesh=bpy.data.meshes.new('Private spatial seam audit');mesh.from_pydata(points,[],faces);mesh.update()
bm=bmesh.new();bm.from_mesh(mesh);before=len(bm.verts)
bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.0001)
boundary=[];multi=[]
for e in bm.edges:
    row=dict(points=[list(v.co) for v in e.verts],length=e.calc_length(),faces=len(e.link_faces))
    if e.is_boundary:boundary.append(row)
    elif not e.is_manifold:multi.append(row)
result=dict(status='DIAGNOSTIC_NOT_APPROVAL',model_sha256=sha(w/'model.blend'),nodes=sorted(nodes),
            upper_surface_winding=upward_audit,input_vertices=before,welded_vertices=len(bm.verts),weld_distance=.0001,
            boundary_edges=boundary,nonmanifold_nonboundary_edges=multi,
            degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces),
            limitations=['Terrain has intentional original outer boundaries; boundary count alone is not a defect classification.',
                         'Storage vertices are welded only in this discarded diagnostic mesh. No geometry saved.'])
bm.free();(w/'inspection/regional-seam-audit.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:len(result[k]) if isinstance(result[k],list) else result[k] for k in ['input_vertices','welded_vertices','boundary_edges','nonmanifold_nonboundary_edges','degenerate_faces']}))
