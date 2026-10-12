"""Audit saved physical leaf triangles without loading a renderer or altering inputs."""
import collections
import hashlib
import json
import struct
import sys
from pathlib import Path
import numpy as np


def audit(path):
    raw = path.read_bytes()
    length = struct.unpack_from('<I', raw, 12)[0]
    doc = json.loads(raw[20:20 + length])
    binary = raw[28 + length:]

    def positions(index):
        accessor = doc['accessors'][index]
        view = doc['bufferViews'][accessor['bufferView']]
        assert accessor['componentType'] == 5126 and accessor['type'] == 'VEC3'
        return np.frombuffer(binary, dtype='<f4', count=accessor['count'] * 3,
                             offset=view.get('byteOffset', 0) + accessor.get('byteOffset', 0)).reshape(-1, 3)

    primitives = doc['meshes'][0]['primitives']
    results = []
    for pose in [-1] + list(range(len(primitives[0].get('targets', [])))):
        edges = collections.defaultdict(list)
        degenerates = 0
        triangles = 0
        for primitive_index, primitive in enumerate(primitives):
            points = positions(primitive['attributes']['POSITION']).copy()
            if pose >= 0:
                points += positions(primitive['targets'][pose]['POSITION'])
            for triangle in points.reshape(-1, 3, 3):
                triangles += 1
                if np.linalg.norm(np.cross(triangle[1] - triangle[0], triangle[2] - triangle[0])) == 0:
                    degenerates += 1
                vertices = [tuple(v.tolist()) for v in triangle]
                for i, j in [(0, 1), (1, 2), (2, 0)]:
                    key = tuple(sorted([vertices[i], vertices[j]]))
                    edges[key].append((primitive_index, 1 if vertices[i] <= vertices[j] else -1))
        results.append(dict(pose='base' if pose < 0 else pose, triangles=triangles,
                            zero_area_triangles=degenerates,
                            edge_multiplicities=dict(collections.Counter(map(len, edges.values()))),
                            unbalanced_directed_edges=sum(sum(x[1] for x in values) != 0 for values in edges.values()),
                            nonmanifold_primitive_combinations={str(k): v for k, v in collections.Counter(
                                tuple(sorted(x[0] for x in values)) for values in edges.values() if len(values) > 2).items()}))
    return dict(status='SAVED_TOPOLOGY_MEASUREMENT_NOT_APPROVAL',
                model=dict(path=str(path), sha256=hashlib.sha256(raw).hexdigest()),
                method='Exact float32 position equality across all leaf primitives; base and each authored morph target. No approximate position weld.',
                results=results)


if __name__ == '__main__':
    path = Path(sys.argv[1])
    result = audit(path)
    output = path.parent / 'saved-topology-audit-v1.json'
    output.write_text(json.dumps(result, indent=2) + '\n')
    print(output)
