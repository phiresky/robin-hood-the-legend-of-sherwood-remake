"""Saved tree native coverage, topology and inferred crown extent evidence."""
import json
import sys
from pathlib import Path
import bpy
import bmesh

sys.path.insert(0,str(Path(__file__).parent))
import audit_native_coverage
from evidence_io import sha
from prepare_props import SIN,COS


def main():
    release=audit_native_coverage.release
    audit_native_coverage.release=lambda:None
    audit_native_coverage.main()
    args=sys.argv[sys.argv.index('--')+1:];worker=Path(args[0]).resolve()
    cfg=json.loads((worker/'workspace.json').read_text());reports=[]
    for obj in bpy.data.collections[cfg['collection_name']].all_objects:
        if obj.type!='MESH' or obj.get('asset_group')!=cfg['asset_id']:continue
        bm=bmesh.new();bm.from_mesh(obj.data);remaining=set(bm.verts);components=[]
        while remaining:
            queue=[remaining.pop()];count=0
            while queue:
                vert=queue.pop();count+=1
                for edge in vert.link_edges:
                    other=edge.other_vert(vert)
                    if other in remaining:remaining.remove(other);queue.append(other)
            components.append(count)
        points=[obj.matrix_world@v.co for v in obj.data.vertices]
        bounds=[[min(p[i] for p in points),max(p[i] for p in points)] for i in range(3)]
        reports.append(dict(source_node=obj.get('source_node'),vertices=len(bm.verts),faces=len(bm.faces),connected_components=components,nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces),bounds=bounds,depth_width_ratio=(bounds[1][1]-bounds[1][0])/(bounds[0][1]-bounds[0][0]),native_projected_y_bounds=[min(-p.y*SIN-p.z*COS for p in points),max(-p.y*SIN-p.z*COS for p in points)]));bm.free()
    (worker/'inspection/saved-tree-geometry.json').write_text(json.dumps(dict(model_sha256=sha(worker/'model.blend'),meshes=reports,limitations=['Thin separate crown leaves intentionally have open edges; wood needs closed nondegenerate surfaces.','Source-frame crop does not terminate geometry. Crown bounds document the hidden depth hypothesis.']),indent=2)+'\n')
    release()


if __name__=='__main__':main()
