"""Measure whether the inferred upper stone meets either lower stone."""
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import write_json,sha

def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]).resolve()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
        objects={o.get('source_node'):o for o in bpy.data.collections['Croisement03 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement03-east-tree-rocks'}
        lower=objects['building-074'];upper=objects['building-075']
        vertices=[lower.matrix_world@v.co for v in lower.data.vertices]
        bvh=BVHTree.FromPolygons(vertices,[list(p.vertices) for p in lower.data.polygons])
        points=[upper.matrix_world@v.co for v in upper.data.vertices]
        bottom=min(p.z for p in points);rows=[]
        for i,point in enumerate(points):
            if point.z>bottom+3:continue
            hit=bvh.ray_cast(Vector((point.x,point.y,100)),Vector((0,0,-1)))
            rows.append(dict(vertex=i,point=list(point),lower_surface_z=hit[0].z if hit[0] is not None else None,vertical_gap=point.z-hit[0].z if hit[0] is not None else None))
        # Intersection is evidence of contact, not proof of physical stability.
        # Empty rays must stay empty rather than becoming fabricated support.
        supported=[r for r in rows if r['vertical_gap'] is not None and r['vertical_gap']<=.001]
        ground=[p for p in points if abs(p.z)<.001]
        area=max((((b-a).cross(c-a)).length/2 for a in ground for b in ground for c in ground),default=0.)
        seated=len(ground)>=3 and area>1 and bottom>=-.001
        write_json(worker/'inspection/upper-stone-support.json',dict(model_sha256=sha(worker/'model.blend'),bottom_z=bottom,probes=rows,supported_probes=len(supported),ground_contact_vertices=len(ground),ground_contact_triangle_area=area,status='GROUND CONTACT PRESENT' if seated else ('LOWER STONE CONTACT PRESENT; inspect extent and stability' if supported else 'HOLD: no sampled support'),limitations=['Sparse low-vertex probes are not a complete collision or stability proof.','Ground is a diagnostic Z0 plane, not completed native terrain.']))
    finally:release()

if __name__=='__main__':main()
