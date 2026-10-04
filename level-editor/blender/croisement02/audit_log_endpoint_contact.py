"""Audit saved rigid log solids against their exact bank and source projection."""
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import SIN, COS


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def planar_triangle(points):
    a, b, c = points
    normal = np.cross(np.array(b,dtype=np.float64)-np.array(a,dtype=np.float64), np.array(c,dtype=np.float64)-np.array(a,dtype=np.float64))
    if abs(normal[2]) < 1e-8:
        return None
    return (list(points) if normal[2] > 0 else [a,c,b], normal, float(np.dot(normal,np.array(a,dtype=np.float64))))


def overlap(subject, clip):
    """Convex triangle clipping, retaining all extrema of linear height gaps."""
    points = [(p.x,p.y) for p in subject]
    for a,b in zip(clip,clip[1:]+clip[:1]):
        if not points:
            break
        def side(p):
            return (b.x-a.x)*(p[1]-a.y)-(b.y-a.y)*(p[0]-a.x)
        result = []
        previous = points[-1]
        previous_side = side(previous)
        for current in points:
            current_side = side(current)
            if (current_side >= 0) != (previous_side >= 0):
                t = previous_side/(previous_side-current_side)
                result.append((previous[0]+t*(current[0]-previous[0]), previous[1]+t*(current[1]-previous[1])))
            if current_side >= 0:
                result.append(current)
            previous,previous_side = current,current_side
        points = result
    return points


def main():
    candidate = OUT / (sys.argv[sys.argv.index('--candidate')+1] if '--candidate' in sys.argv else 'log-trap-state-candidate-v8')
    state = sys.argv[sys.argv.index('--state')+1] if '--state' in sys.argv else 'applied'
    assert state in ('covered', 'applied')
    manifest = json.loads((candidate / 'manifest.json').read_text())
    bank = OUT / 'terrain-bank-candidate/assets/croisement02-north-woodland-bank'
    audit = json.loads((bank / 'inspection/saved-model-audit.json').read_text())
    assert sha(bank / 'model.blend') == audit['model_sha256']
    assert sha(candidate / 'worker.blend') == manifest['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(candidate / 'worker.blend'))
    logs = sorted((o for o in bpy.data.objects if o.get('state_endpoint') == state), key=lambda o: o.name)
    names = [r['object'] for r in audit['objects'] if r['source_node'] in [f'building-{i:03d}' for i in range(5)]]
    with bpy.data.libraries.load(str(bank / 'model.blend'), link=False) as (src, dst):
        dst.objects = names
    for obj in dst.objects:
        bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.update()
    vertices, faces = [], []
    for obj in dst.objects:
        offset = len(vertices)
        vertices.extend(obj.matrix_world @ v.co for v in obj.data.vertices)
        faces.extend(tuple(offset + j for j in p.vertices) for p in obj.data.polygons)
    bvh = BVHTree.FromPolygons(vertices, faces)
    bank_triangles = []
    for obj in dst.objects:
        obj.data.calc_loop_triangles()
        exact_worst = None
        negligible_overlap_count = 0
        for triangle in obj.data.loop_triangles:
            plane = planar_triangle([obj.matrix_world @ obj.data.vertices[i].co for i in triangle.vertices])
            if plane:
                bank_triangles.append(plane)
    log_bvhs = [BVHTree.FromPolygons([o.matrix_world @ v.co for v in o.data.vertices], [tuple(p.vertices) for p in o.data.polygons]) for o in logs] if state == 'covered' else []
    records = []
    for index, obj in enumerate(logs):
        obj.data.calc_loop_triangles()
        clearances = []
        worst = None
        lower_contacts = [float('inf')] * index if state == 'covered' else []
        # Barycentric surface lattice covers the actual saved polygonal solid,
        # including caps and edges, rather than a circular proxy for the log.
        for triangle in obj.data.loop_triangles:
            a, b, c = [obj.matrix_world @ obj.data.vertices[i].co for i in triangle.vertices]
            for i in range(41):
                for j in range(41-i):
                    p = a + (b-a)*(i/40) + (c-a)*(j/40)
                    hit = bvh.ray_cast(Vector((p.x, p.y, 500)), Vector((0, 0, -1)), 1000)
                    surface = max(0, hit[0].z) if hit[0] is not None else 0
                    clearance = float(p.z-surface)
                    for lower in range(len(lower_contacts)):
                        nearest = log_bvhs[lower].find_nearest(p)
                        if nearest[0] is not None:
                            lower_contacts[lower] = min(lower_contacts[lower], nearest[3])
                    clearances.append(clearance)
                    if worst is None or clearance < worst['clearance']:
                        worst = dict(clearance=clearance, point=list(p), receiver_height=surface)
        uv_error = 0.0
        for polygon in obj.data.polygons:
            for loop in polygon.loop_indices:
                p = obj.matrix_world @ obj.data.vertices[obj.data.loops[loop].vertex_index].co
                uv = obj.data.uv_layers['Native target projection'].data[loop].uv
                uv_error = max(uv_error, abs((p.x-390)-uv.x*256), abs((-p.y*SIN-p.z*COS-441)-(1-uv.y)*237))
        row = manifest['surveys']['applied' if state == 'applied' else 'initial'][index]
        ax, ay, bx, by, radius, za, *other = row
        zb = other[0] if other else za
        source_top = 441 if state == 'applied' else 453
        # The algebraic source-ray lift must preserve both source endpoints.
        endpoint_error = max(abs(-(-(y+source_top+z*COS)/SIN)*SIN-z*COS-(y+source_top)) for y,z in [(ay,za),(by,zb)])
        exact_minimum = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
        exact_worst = None
        negligible_overlap_count = 0
        for triangle in obj.data.loop_triangles:
            plane = planar_triangle([obj.matrix_world @ obj.data.vertices[i].co for i in triangle.vertices])
            if plane is None:
                continue
            points,normal,offset = plane
            for bank_points,bank_normal,bank_offset in bank_triangles:
                if any(max(p[axis] for p in points) < min(p[axis] for p in bank_points) or max(p[axis] for p in bank_points) < min(p[axis] for p in points) for axis in (0,1)):
                    continue
                intersection = overlap(points,bank_points)
                intersection_area = abs(sum(a[0]*b[1]-b[0]*a[1] for a,b in zip(intersection,intersection[1:]+intersection[:1])))/2
                # Subpixel floating-point corner slivers are not volumetric penetration.
                if intersection_area < 1e-5:
                    negligible_overlap_count += bool(intersection)
                    continue
                for x,y in intersection:
                    log_z = (offset-normal[0]*x-normal[1]*y)/normal[2]
                    bank_z = (bank_offset-bank_normal[0]*x-bank_normal[1]*y)/bank_normal[2]
                    if log_z-bank_z < exact_minimum:
                        exact_minimum = log_z-bank_z
                        hit = bvh.ray_cast(Vector((x,y,500)),Vector((0,0,-1)),1000)
                        exact_worst = dict(x=x,y=y,log_z=log_z,bank_z=bank_z,overlap_area=intersection_area,log_normal_z_ratio=float(abs(normal[2])/np.linalg.norm(normal)),bank_normal_z_ratio=float(abs(bank_normal[2])/np.linalg.norm(bank_normal)),ray_bank_z=hit[0].z if hit[0] is not None else None)
        records.append(dict(object=obj.name, lower_log_sampled_surface_distances=lower_contacts, lower_log_surface_intersections=[len(log_bvhs[index].overlap(log_bvhs[j])) for j in range(index)] if state == 'covered' else [], samples=len(clearances), minimum_clearance=min(clearances), exact_piecewise_surface_minimum_clearance=exact_minimum, exact_worst=exact_worst, ignored_corner_slivers=negligible_overlap_count, maximum_clearance=max(clearances), penetrating_samples=sum(x < -.05 for x in clearances), worst=worst, native_uv_projection_error_pixels=uv_error, endpoint_source_ray_error_pixels=endpoint_error))
    report = dict(status='HOLD' if any(r['exact_piecewise_surface_minimum_clearance'] < -.05 or (r['exact_piecewise_surface_minimum_clearance'] > 1.05 and not (r['lower_log_sampled_surface_distances'] and min(r['lower_log_sampled_surface_distances']) <= .6)) or any(r['lower_log_surface_intersections']) for r in records) else 'sampled support pass; source and foreground review still required', state=state, model_sha256=manifest['model_sha256'], bank_model_sha256=audit['model_sha256'], records=records, projected_overlap_area_tolerance=1e-5, limitations=['Triangle-overlap extrema check continuous bank/ground clearance. Covered lower-log distances are sampled and surface intersections checked; stable balance is not proven.', 'Independent endpoints remain unapproved geometric hypotheses; no motion identities inferred.'])
    (candidate / ('dense-contact-audit.json' if state == 'applied' else 'covered-contact-audit.json')).write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
