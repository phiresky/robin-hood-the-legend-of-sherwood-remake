"""Stage bridge timber as extruded mesh triangles, preserving profile openings.

Run from level-editor. Requires NumPy/SciPy; reads only the published asset GLB.
The resulting convex pieces are candidates, not publication or gameplay proof.
"""
import hashlib
import json
import math
import struct
from pathlib import Path

import numpy as np
from scipy.spatial import ConvexHull

path = Path('library/3d-assets/croisement03/croisement03-timber-bridge/model.glb')
data = path.read_bytes()
length = struct.unpack_from('<I', data, 12)[0]
gltf = json.loads(data[20:20 + length])
binary = data[28 + length:]


def accessor(index):
    a = gltf['accessors'][index]
    view = gltf['bufferViews'][a['bufferView']]
    width = {'VEC3': 3, 'SCALAR': 1}[a['type']]
    dtype = {5123: '<u2', 5125: '<u4', 5126: '<f4'}[a['componentType']]
    assert not a.get('sparse')
    assert view.get('byteStride', np.dtype(dtype).itemsize * width) == np.dtype(dtype).itemsize * width
    return np.frombuffer(binary, dtype=dtype,
                         offset=view.get('byteOffset', 0) + a.get('byteOffset', 0),
                         count=a['count'] * width).reshape(-1, width)


node = next(n for n in gltf['nodes'] if n.get('name') == 'scenery-croisement03-timber-bridge')
assert len(node['children']) == 1
child = gltf['nodes'][node['children'][0]]
for part in [node, child]:
    assert not any(key in part for key in ['matrix', 'translation', 'rotation', 'scale'])
primitives = gltf['meshes'][child['mesh']]['primitives']
assert len(primitives) == 1
primitive = primitives[0]
assert primitive.get('mode', 4) == 4
vertices = accessor(primitive['attributes']['POSITION']).astype(float)
faces = accessor(primitive['indices']).reshape(-1, 3)
parents = list(range(len(vertices)))


def root(i):
    while parents[i] != i:
        parents[i] = parents[parents[i]]
        i = parents[i]
    return i


seen = {}
for i, point in enumerate(vertices):
    key = tuple(point)
    if key in seen:
        parents[root(i)] = root(seen[key])
    else:
        seen[key] = i
for a, b, c in faces:
    parents[root(a)] = root(b)
    parents[root(b)] = root(c)
groups = {}
for i, face in enumerate(faces):
    groups.setdefault(root(face[0]), []).append(i)

components = []
pieces = []
ray_probes = []
mesh = vertices[faces]
edge1 = mesh[:, 1] - mesh[:, 0]
edge2 = mesh[:, 2] - mesh[:, 0]


def sample_rays(points, center, normal, axes, component):
    coordinates = (points - center) @ axes[:, 1:]
    low, high = coordinates.min(axis=0), coordinates.max(axis=0)
    distances = (points - center) @ normal
    candidates = {True: [], False: []}
    for u in np.linspace(low[0], high[0], 11)[1:-1]:
        for v in np.linspace(low[1], high[1], 11)[1:-1]:
            base = center + axes[:, 1] * u + axes[:, 2] * v
            a = base + normal * (distances.min() - 1)
            b = base + normal * (distances.max() + 1)
            direction = b - a
            h = np.cross(np.broadcast_to(direction, edge2.shape), edge2)
            determinant = np.einsum('ij,ij->i', edge1, h)
            usable = abs(determinant) > 1e-10
            inverse = np.divide(1., determinant, out=np.zeros_like(determinant), where=usable)
            s = a - mesh[:, 0]
            bary_u = inverse * np.einsum('ij,ij->i', s, h)
            q = np.cross(s, edge1)
            bary_v = inverse * (q @ direction)
            t = inverse * np.einsum('ij,ij->i', edge2, q)
            hit = usable & (bary_u >= 0) & (bary_v >= 0) & (bary_u + bary_v <= 1) & (t > 0) & (t < 1)
            clear = not bool(hit.any())
            endpoints = np.array([a, b]) * [1, -math.sin(math.radians(35)), math.cos(math.radians(35))]
            candidates[clear].append(endpoints.tolist())
    assert candidates[False], 'Component sampling must hit its mesh'
    for clear, endpoints in candidates.items():
        for i in np.linspace(0, len(endpoints) - 1, min(8, len(endpoints)), dtype=int):
            ray_probes.append({'name': f'mesh-{component}-{clear}-{i}', 'clear': clear,
                               'endpoints': endpoints[i]})


def emit(points, component, face_id):
    game = points * [1, -math.sin(math.radians(35)), math.cos(math.radians(35))]
    hull = ConvexHull(game)
    pieces.append({
        'id': f'timber-{component}-face-{face_id}',
        'component': component, 'meshFace': face_id,
        'vertices': game.tolist(), 'volume': hull.volume,
        'faces': [{'indices': f.tolist(), 'plane': p.tolist()}
                  for f, p in zip(hull.simplices, hull.equations)],
    })


def convex_caps(front, center, axes):
    """Merge adjacent cap triangles only when their union is already convex."""
    polygons = {face: [tuple(p) for p in vertices[faces[face]]] for face in front}
    areas = {}
    for key, points in polygons.items():
        hull = ConvexHull((np.array(points) - center) @ axes[:, 1:])
        areas[key] = hull.volume
        polygons[key] = [points[i] for i in hull.vertices]
    while True:
        edges = {}
        merged = False
        for key, polygon in list(polygons.items()):
            for a, b in zip(polygon, polygon[1:] + polygon[:1]):
                edge = tuple(sorted([a, b]))
                other = edges.get(edge)
                if other is not None:
                    points = list(dict.fromkeys(polygons[other] + polygon))
                    hull = ConvexHull((np.array(points) - center) @ axes[:, 1:])
                    expected = areas[other] + areas[key]
                    if abs(hull.volume - expected) <= max(1e-8, expected * 1e-10):
                        polygons[other] = [points[i] for i in hull.vertices]
                        areas[other] = hull.volume
                        del polygons[key]
                        del areas[key]
                        merged = True
                        break
                edges[edge] = key
            if merged:
                break
        if not merged:
            return [(key, np.array(polygon)) for key, polygon in polygons.items()]


for face_ids in sorted(groups.values(), key=min):
    points = np.unique(vertices[faces[face_ids].reshape(-1)], axis=0)
    if min(face_ids) == 0:
        assert len(face_ids) == 12
        continue  # Deck top/bottom is authored separately by the staging script.
    center = points.mean(axis=0)
    _, axes = np.linalg.eigh(np.cov((points - center).T))
    normal = axes[:, 0]
    # These rail profiles are vertical extrusions. Remove only the tiny tilt
    # introduced by mesh export; otherwise near-vertical faces yield unstable
    # height planes when partitioned into runtime solids.
    if abs(normal[2]) < 1e-5:
        normal[2] = 0
        normal /= np.linalg.norm(normal)
    distances = (points - center) @ normal
    sample_rays(points, center, normal, axes, min(face_ids))
    low, high = distances.min(), distances.max()
    fit_error = np.minimum(abs(distances - low), abs(distances - high)).max()
    mesh_triangles = vertices[faces[face_ids]] - center
    mesh_volume = abs(np.einsum('ij,ij->i', mesh_triangles[:, 0],
                      np.cross(mesh_triangles[:, 1], mesh_triangles[:, 2])).sum() / 6)
    first_piece = len(pieces)
    if fit_error >= 0.001:
        assert len(face_ids) == 12, 'Review non-extruded component'
        hull = ConvexHull(points)
        assert abs(hull.volume - mesh_volume) < mesh_volume * 1e-5
        emit(points, min(face_ids), min(face_ids))
        components.append({'firstFace': min(face_ids), 'faces': len(face_ids),
                           'kind': 'convex-mesh', 'fitError': 0, 'meshVolume': mesh_volume})
        continue
    assert fit_error < 0.001, (min(face_ids), 'component is not a thin extrusion', fit_error)
    thickness = high - low
    assert 0 < thickness < 4
    front = []
    for face_id in face_ids:
        tri = vertices[faces[face_id]]
        if np.max(abs((tri - center) @ normal - high)) < 0.001:
            front.append(face_id)
    assert front
    caps = convex_caps(front, center, axes)
    for face_id, points in caps:
        # Fit both caps to the component planes, within the measured error.
        cap = points + np.outer(high - (points - center) @ normal, normal)
        prism = np.concatenate([cap, cap - thickness * normal])
        emit(prism, min(face_ids), face_id)
    recovered_volume = sum(p['volume'] for p in pieces[first_piece:])
    expected_volume = mesh_volume * math.sin(math.radians(35)) * math.cos(math.radians(35))
    assert abs(recovered_volume - expected_volume) < expected_volume * 0.0001
    components.append({'firstFace': min(face_ids), 'faces': len(face_ids),
                       'capFaces': front, 'thickness': thickness, 'fitError': fit_error,
                       'convexCaps': len(caps),
                       'meshVolume': mesh_volume, 'recoveredVolume': recovered_volume})

output = Path('work/map-compile/croisement-bridge-structure.json')
output.write_text(json.dumps({'modelSha256': hashlib.sha256(data).hexdigest(),
                              'components': components, 'pieces': pieces, 'rayProbes': ray_probes}))
print(json.dumps({'output': str(output), 'components': len(components), 'pieces': len(pieces),
                  'maximumFitError': max(c['fitError'] for c in components)}))
