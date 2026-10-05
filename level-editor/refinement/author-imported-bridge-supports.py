"""Stage mesh-owned convex support solids, with explicit grounded feet.

Run from level-editor: python3 refinement/author-imported-bridge-supports.py
Requires NumPy and SciPy. Output remains unpublished under work/map-compile.
"""
import hashlib
import json
import math
import struct
from pathlib import Path
import numpy as np
from scipy.spatial import ConvexHull

base = Path('library/3d-assets/sketchfab/sketchfab-long-wood-bridge')
review = json.loads(Path('refinement/catalogs/sketchfab-long-wood-bridge-deck-review.json').read_text())
data = (base / 'model.glb').read_bytes()
assert hashlib.sha256(data).hexdigest() == review['modelSha256']
length = struct.unpack_from('<I', data, 12)[0]
gltf = json.loads(data[20:20 + length])
binary = data[28 + length:]
def accessor(index):
    a = gltf['accessors'][index]
    view = gltf['bufferViews'][a['bufferView']]
    width = {'VEC3': 3, 'SCALAR': 1}[a['type']]
    dtype = {5123: '<u2', 5125: '<u4', 5126: '<f4'}[a['componentType']]
    assert view.get('byteStride', np.dtype(dtype).itemsize * width) == np.dtype(dtype).itemsize * width
    return np.frombuffer(binary, dtype=dtype, offset=view.get('byteOffset', 0) + a.get('byteOffset', 0), count=a['count'] * width).reshape(-1, width)
node = next(n for n in gltf['nodes'] if n.get('name') == review['node'])
primitive = gltf['meshes'][node['mesh']]['primitives'][review['primitive']]
vertices = accessor(primitive['attributes']['POSITION']).astype(float)
faces = accessor(primitive['indices']).reshape(-1, 3)
parents = list(range(len(vertices)))
def root(i):
    while parents[i] != i:
        parents[i] = parents[parents[i]]
        i = parents[i]
    return i
def union(a, b):
    parents[root(a)] = root(b)
seen = {}
for i, point in enumerate(vertices):
    key = tuple(point)
    if key in seen:
        union(i, seen[key])
    else:
        seen[key] = i
for a, b, c in faces:
    union(a, b)
    union(b, c)
groups = {}
for i in range(len(vertices)):
    groups.setdefault(root(i), []).append(i)
supports = [ids for ids in groups.values() if vertices[ids, 2].min() < 2 and vertices[ids, 2].max() > 80]
assert len(supports) == 12
supports.sort(key=lambda ids: tuple(vertices[ids].mean(axis=0)))
result = []
for index, ids in enumerate(supports):
    points = vertices[ids].copy()
    assert len(points) == 21
    # Mesh foot pads differ by less than two scene units. Extend only those
    # low pad vertices to the asset foundation, leaving the inclined shaft.
    grounded = np.nonzero(points[:, 2] < 2)[0]
    original_floor = points[grounded, 2].tolist()
    points[grounded, 2] = 0
    points *= [1, -math.sin(math.radians(35)), math.cos(math.radians(35))]
    points = np.unique(points, axis=0)
    hull = ConvexHull(points)
    result.append({
        'id': f'support-{index:02}',
        'meshVertexIndices': ids,
        'groundedSceneHeights': original_floor,
        'vertices': points.tolist(),
        'volume': hull.volume,
        'faces': [{'indices': face.tolist(), 'plane': plane.tolist()} for face, plane in zip(hull.simplices, hull.equations)],
    })
output = Path('work/map-compile/imported-bridge-support-hulls.json')
output.write_text(json.dumps({'modelSha256': review['modelSha256'], 'supports': result}, indent=2) + '\n')
print(json.dumps({'output': str(output), 'supports': len(result), 'hullFaces': sum(len(s['faces']) for s in result)}))
