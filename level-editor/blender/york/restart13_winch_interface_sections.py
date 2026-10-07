"""Export exact chain/support crossings and small orthographic section evidence."""
import hashlib, json, math, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2/winch-shaft-return-candidate-v1'
OUT = BASE / 'interface-sections-v1'
if OUT.exists():
    raise FileExistsError(OUT)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
bpy.ops.wm.open_mainfile(filepath=str(BASE / 'model.blend'))
scene = bpy.context.scene
scene.frame_set(88)
bpy.context.view_layer.update()
links = [o for o in scene.objects if o.name.startswith('Shaft return closed link')]
body = [o for o in scene.objects if o.type == 'MESH' and o.get('native_patch') == 'patch-004' and o not in links and not o.name.startswith('Inferred upper')]

def triangles(objects):
    vertices, faces, owners = [], [], []
    for o in objects:
        start = len(vertices)
        vertices.extend(o.matrix_world @ v.co for v in o.data.vertices)
        o.data.calc_loop_triangles()
        for f in o.data.loop_triangles:
            faces.append(tuple(start + i for i in f.vertices))
            owners.append(o.name)
    values = np.asarray(vertices)
    return BVHTree.FromPolygons(vertices, faces, all_triangles=True), values[np.asarray(faces)], owners

def crossing_points(a, b):
    result = []
    e1, e2 = b[1] - b[0], b[2] - b[0]
    for i in range(3):
        origin, direction = a[i], a[(i + 1) % 3] - a[i]
        p = np.cross(direction, e2)
        det = np.dot(e1, p)
        if abs(det) < 1e-8:
            continue
        t = origin - b[0]
        u = np.dot(t, p) / det
        q = np.cross(t, e1)
        v = np.dot(direction, q) / det
        distance = np.dot(e2, q) / det
        if u >= -1e-6 and v >= -1e-6 and u + v <= 1 + 1e-6 and 1e-6 < distance < 1 - 1e-6:
            result.append((origin + direction * distance).tolist())
    return result

ctree, ct, cnames = triangles(links)
btree, bt, bnames = triangles(body)
crossings = []
for ci, bi in ctree.overlap(btree):
    points = crossing_points(ct[ci], bt[bi]) + crossing_points(bt[bi], ct[ci])
    if points:
        crossings.append({'body': bnames[bi], 'link': cnames[ci], 'points': points})
counts = {}
for row in crossings:
    counts[row['body']] = counts.get(row['body'], 0) + 1
objects = []
for o in body + links:
    points = [o.matrix_world @ v.co for v in o.data.vertices]
    # The chain above the body has no bearing on the entry cross sections.
    if min(v.z * c for v in points) > 120:
        continue
    objects.append({'name': o.name, 'vertices': [list(v) for v in points], 'faces': [list(p.vertices) for p in o.data.polygons]})
OUT.mkdir()
payload = {
    'status': 'Private exact intersection diagnosis; no replacement geometry saved',
    'model_sha256': hashlib.sha256((BASE / 'model.blend').read_bytes()).hexdigest(),
    'body_crossing_triangle_pair_counts': counts,
    'crossings': crossings,
    'objects': objects,
    'projection': {'sine': s, 'cosine': c, 'native': ['world_x', '-world_y * sine - world_z * cosine']},
    'limitations': ['Exact triangle-edge intersections do not detect full containment.', 'Existing return route and support depth are hypotheses; this packet does not establish either as source-authoritative.'],
}
(OUT / 'geometry.json').write_text(json.dumps(payload) + '\n')
print(json.dumps({'path': str(OUT), 'crossing_pairs': counts, 'objects': len(objects)}))
