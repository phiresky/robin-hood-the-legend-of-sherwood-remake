"""Read-only source-ray depth comparison for rock pixels occluded by terrain."""
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY


def load_surface(model,asset):
    bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
    vertices=[];triangles=[];objects=[]
    for obj in bpy.data.objects:
        if obj.type!='MESH' or obj.get('asset_group')!=asset:continue
        start=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);obj.data.calc_loop_triangles();triangles.extend(tuple(start+i for i in t.vertices) for t in obj.data.loop_triangles)
        objects.append(dict(name=obj.name,matrix_world=[list(r) for r in obj.matrix_world]))
    return BVHTree.FromPolygons(vertices,triangles,all_triangles=True),np.array(vertices),objects


def main():
    root=OUT/'restart2-bank321';output=root/'northwest-rock-contact-v1'
    if output.exists():raise FileExistsError(output)
    output.mkdir()
    bank=root/'foot-candidate-v2/worker.blend';rock=OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/bake-single-v2/worker.blend'
    audit_path=OUT/'restart2-vegetation/neutral-first-hit-v1/audit.json';audit=json.loads(audit_path.read_text())
    samples=next(c for c in audit['components'] if c['component']==19)['samples']
    acquire()
    try:
        bank_tree,bank_vertices,bank_objects=load_surface(bank,'croisement02-north-woodland-bank')
        rock_tree,rock_vertices,rock_objects=load_surface(rock,'croisement02-northwest-rock-outcrop')
        rows=[]
        for sample in samples:
            x,y=sample['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+Vector(RAY)*10000
            b,_,_,bd=bank_tree.ray_cast(origin,-Vector(RAY));r,_,_,rd=rock_tree.ray_cast(origin,-Vector(RAY))
            rows.append(dict(pixel=[x,y],bank_hit=list(b) if b is not None else None,rock_hit=list(r) if r is not None else None,bank_distance=bd,rock_distance=rd,bank_in_front=b is not None and(r is None or bd<rd),required_rock_source_ray_shift=float(rd-bd) if b is not None and r is not None else None))
        differences=[r['required_rock_source_ray_shift'] for r in rows if r['required_rock_source_ray_shift'] is not None]
        write_json(output/'report.json',dict(status='Read-only depth diagnosis; no placement change',bank_model=str(bank),bank_model_sha256=sha(bank),rock_model=str(rock),rock_model_sha256=sha(rock),frozen_audit_sha256=sha(audit_path),sample_pixels=len(rows),bank_in_front=sum(r['bank_in_front'] for r in rows),missing_rock=sum(r['rock_hit'] is None for r in rows),required_source_ray_shift_range=[min(differences),max(differences)] if differences else None,bank_world_bounds=[bank_vertices.min(axis=0).tolist(),bank_vertices.max(axis=0).tolist()],rock_world_bounds=[rock_vertices.min(axis=0).tolist(),rock_vertices.max(axis=0).tolist()],bank_objects=bank_objects,rock_objects=rock_objects,rows=rows,geometry_changed=False,source_ownership_changed=False))
        print(json.dumps({'pixels':len(rows),'bank_in_front':sum(r['bank_in_front'] for r in rows),'missing_rock':sum(r['rock_hit'] is None for r in rows),'depth_shift_range':[min(differences),max(differences)] if differences else None}))
    finally:release()


if __name__=='__main__':main()
