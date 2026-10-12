"""Test the unchanged double-precision CPU trace sample set against saved bank triangles."""
import json
import math
from pathlib import Path
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from restart2_bank_boundary_ray import boundary_hits


def verify(scene, folder, sha):
    interface = json.loads((folder / 'interface-construction.json').read_text())
    path = Path(interface['geometry_plan'])
    assert sha(path) == interface['geometry_plan_sha256']
    data = json.loads(path.read_text())
    west = interface.get('private_revised53')
    if west:
        path = Path(west['path'])
        assert sha(path) == west['sha256']
        data['53'] = json.loads(path.read_text())
    s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
    ray = Vector((0, -c, s))
    trees, arrays = {}, {}
    for key in ('52', '53', '54'):
        obj = scene.objects['Candidate bank ' + key]
        obj.data.calc_loop_triangles()
        vertices = [obj.matrix_world @ v.co for v in obj.data.vertices]
        triangles = [list(t.vertices) for t in obj.data.loop_triangles]
        trees[key] = BVHTree.FromPolygons(vertices, triangles, all_triangles=True)
        arrays[key] = ([tuple(v) for v in vertices], triangles)
    failures, count, max_error = [], 0, 0.0
    for key in ('52', '54') + (('53',) if west else ()):
        mesh = data[key]
        for trace in mesh['traces']:
            points = np.array(trace.get('source_path_world', [mesh['vertices'][i] for i in trace['vertices']]), dtype=np.float64)
            for a, b in zip(points, points[1:]):
                distance = np.linalg.norm([b[0]-a[0], -(b[1]-a[1])*s-(b[2]-a[2])*c])
                for t in np.linspace(0, 1, max(2, int(distance*2)+1)):
                    p = a*(1-t)+b*t
                    point = Vector(p)
                    count += 1
                    error = trees[key].find_nearest(point)[3]
                    max_error = max(max_error, error)
                    hits = []
                    for other, tree in trees.items():
                        hit, _, _, d = tree.ray_cast(point+ray*5000, -ray)
                        if hit is not None:
                            hits.append((d, other))
                    if not hits:
                        precise = boundary_hits([float(point[j])+float(ray[j])*5000 for j in range(3)], [-float(x) for x in ray], arrays)
                        hits = [(h['distance'], h['owner']) for h in precise]
                    first = min(hits) if hits else None
                    if error > 1e-4 or first is None or first[0] < 4999.99:
                        failures.append(dict(owner=key, trace=trace['id'], source=[float(p[0]), float(-p[1]*s-p[2]*c)], surface_error=error, firsthit=first))
    report = dict(status='PASS_EXACT_CPU_TRACE_SET' if not failures else 'HOLD', samples=count, failures=failures, maximum_surface_error=max_error, model_sha256=sha(folder/'worker.blend'), limits=['Exact CPU double-coordinate interpolation counts, converted to saved float32 ray coordinates only after interpolation.', 'Source-facing coverage does not prove final volume anatomy or complete material ownership.'])
    (folder/'exact-cpu-trace-saved-guard.json').write_text(json.dumps(report, indent=2)+'\n')
    assert count == 1834 and not failures, report
    return report
