"""Audit cart clearance against current evaluated bank and ground geometry."""
import json,sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from audit_log_endpoint_contact import planar_triangle,overlap
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart2_rebind_state_receivers import MODELS


def audit(worker,dest):
    metadata=json.loads((worker/'manifest.json').read_text());model=worker/'worker.blend';assert sha(model)==metadata['model_sha256']
    receiver_vertices=[];receiver_faces=[];bank_planes=[];receivers=[]
    for label,path,digest in MODELS:
        assert sha(path)==digest
        bpy.ops.wm.open_mainfile(filepath=str(path))
        if label=='bank':
            ws=json.loads((path.parent/'workspace.json').read_text());bpy.context.window.scene=bpy.data.scenes[ws['scene_name']]
            selected=[o for o in bpy.data.collections[ws['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==ws['asset_id']]
            assert len(selected)==5
        else:selected=[bpy.data.objects['Croisement02 Terrain']]
        bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get()
        for obj in selected:
            evaluated=obj.evaluated_get(deps);data=evaluated.to_mesh();data.calc_loop_triangles();matrix=evaluated.matrix_world.copy()
            vertices=[matrix@v.co for v in data.vertices];faces=[tuple(t.vertices) for t in data.loop_triangles]
            start=len(receiver_vertices);receiver_vertices.extend(vertices);receiver_faces.extend(tuple(start+i for i in face) for face in faces)
            if label=='bank':
                for face in faces:
                    plane=planar_triangle([vertices[i] for i in face])
                    if plane:bank_planes.append(plane)
            else:assert max(abs(v.z) for v in vertices)<.001,'Ground is not flat'
            receivers.append(dict(receiver=label,model_sha256=digest,object=obj.name,matrix_world=[list(r) for r in matrix],vertices=len(vertices),triangles=len(faces)))
            evaluated.to_mesh_clear()
    tree=BVHTree.FromPolygons(receiver_vertices,receiver_faces,all_triangles=True)
    bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();records=[]
    for obj in bpy.context.scene.objects:
        if obj.type!='MESH':continue
        evaluated=obj.evaluated_get(deps);data=evaluated.to_mesh();data.calc_loop_triangles();matrix=evaluated.matrix_world.copy()
        vertices=[matrix@v.co for v in data.vertices];low=np.min(vertices,axis=0);high=np.max(vertices,axis=0)
        candidates=[p for p in bank_planes if not any(high[a]<min(v[a] for v in p[0]) or max(v[a] for v in p[0])<low[a] for a in (0,1))]
        minimum=float(low[2]);count=0;misses=0;wheel_contacts=[]
        for p in vertices:
            hit=tree.ray_cast(Vector((p.x,p.y,2000)),Vector((0,0,-1)),4000)
            if hit[0] is None:misses+=1
            else:
                clearance=p.z-hit[0].z;minimum=min(minimum,clearance)
                if abs(clearance)<.05:wheel_contacts.append(list(p))
        for triangle in data.loop_triangles:
            plane=planar_triangle([vertices[i] for i in triangle.vertices])
            if not plane:continue
            points,n,c=plane
            for bp,bn,bc in candidates:
                if any(max(p[a] for p in points)<min(p[a] for p in bp) or max(p[a] for p in bp)<min(p[a] for p in points) for a in (0,1)):continue
                polygon=overlap(points,bp);area=abs(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(polygon,polygon[1:]+polygon[:1])))/2
                if area<1e-5:continue
                count+=1
                for x,y in polygon:minimum=min(minimum,(c-n[0]*x-n[1]*y)/n[2]-(bc-bn[0]*x-bn[1]*y)/bn[2])
        records.append(dict(object=obj.name,minimum_clearance=minimum,receiver_misses=misses,bank_surface_overlaps=count,contacts=wheel_contacts,world_bounds=[low.tolist(),high.tolist()]))
        evaluated.to_mesh_clear()
    wheels=[r for r in records if r['object'].startswith('Wheel rim')]
    passed=len(wheels)==4 and all(r['minimum_clearance']>=-.05 and not r['receiver_misses'] for r in records) and all(r['contacts'] for r in wheels)
    dest.mkdir(parents=True,exist_ok=False)
    report=dict(status='PASS' if passed else 'HOLD',model_sha256=sha(model),source_worker=str(worker),receivers=receivers,objects=records,
                method='Reopen receiver source scene, evaluated dependency-graph mesh and matrix; exact projected triangle overlap plus vertex receiver rays',
                limitations=['Initial pose only; no approach, breakup, motion or texture approval.','Contact does not settle native source ownership or structural proportions.'])
    write_json(dest/'report.json',report);print(report['status'],[(r['object'],round(r['minimum_clearance'],4)) for r in records])


def main():
    args=sys.argv[sys.argv.index('--')+1:];acquire()
    try:audit(Path(args[0]),Path(args[1]))
    finally:release()

if __name__=='__main__':main()
