"""Audit saved rigid log solids against their exact bank and source projection."""
import hashlib
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import SIN, COS


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    candidate = OUT / 'log-trap-state-candidate-v8'
    manifest = json.loads((candidate / 'manifest.json').read_text())
    bank = OUT / 'terrain-bank-candidate/assets/croisement02-north-woodland-bank'
    audit = json.loads((bank / 'inspection/saved-model-audit.json').read_text())
    assert sha(bank / 'model.blend') == audit['model_sha256']
    assert sha(candidate / 'worker.blend') == manifest['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(candidate / 'worker.blend'))
    logs = sorted((o for o in bpy.data.objects if o.get('state_endpoint') == 'applied'), key=lambda o: o.name)
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
    records = []
    for index, obj in enumerate(logs):
        obj.data.calc_loop_triangles()
        clearances = []
        worst = None
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
                    clearances.append(clearance)
                    if worst is None or clearance < worst['clearance']:
                        worst = dict(clearance=clearance, point=list(p), receiver_height=surface)
        uv_error = 0.0
        for polygon in obj.data.polygons:
            for loop in polygon.loop_indices:
                p = obj.matrix_world @ obj.data.vertices[obj.data.loops[loop].vertex_index].co
                uv = obj.data.uv_layers['Native target projection'].data[loop].uv
                uv_error = max(uv_error, abs((p.x-390)-uv.x*256), abs((-p.y*SIN-p.z*COS-441)-(1-uv.y)*237))
        ax, ay, bx, by, radius, za, zb = manifest['surveys']['applied'][index]
        # The algebraic source-ray lift must preserve both source endpoints.
        endpoint_error = max(abs(-(-(y+441+z*COS)/SIN)*SIN-z*COS-(y+441)) for y,z in [(ay,za),(by,zb)])
        records.append(dict(object=obj.name, samples=len(clearances), minimum_clearance=min(clearances), maximum_clearance=max(clearances), penetrating_samples=sum(x < -.05 for x in clearances), worst=worst, native_uv_projection_error_pixels=uv_error, endpoint_source_ray_error_pixels=endpoint_error))
    report = dict(status='HOLD' if any(r['minimum_clearance'] < -.05 or r['minimum_clearance'] > 1.05 for r in records) else 'sampled support pass; source and foreground review still required', model_sha256=manifest['model_sha256'], bank_model_sha256=audit['model_sha256'], records=records, limitations=['Dense surface samples do not prove continuous mesh collision absence.', 'Independent endpoints remain unapproved geometric hypotheses; no motion identities inferred.'])
    (candidate / 'dense-contact-audit.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
