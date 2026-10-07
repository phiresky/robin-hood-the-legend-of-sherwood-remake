"""Isolate a touching continuation and its adjacent forks without Boolean repair."""
import hashlib
import json
from collections import Counter
from pathlib import Path

import numpy as np


def audit(vertices, faces):
    triangles = vertices[faces]
    normals = np.cross(triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0])
    edges = Counter(tuple(sorted((int(f[j]), int(f[(j + 1) % 3])))) for f in faces for j in range(3))
    directed = Counter((int(f[j]), int(f[(j + 1) % 3])) for f in faces for j in range(3))
    return dict(nonmanifold_edges=sum(n != 2 for n in edges.values()),
                winding_errors=sum(directed[a, b] != directed[b, a] for a, b in edges),
                zero_area=int((np.linalg.norm(normals, axis=1) < 1e-9).sum()),
                volume=float(np.einsum('ij,ij->i', triangles[:, 0] - vertices.mean(0), normals).sum() / 6))


def main():
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    source = root / 'tree08-v12-chain-cpu-v3'
    output = root / 'tree08-v12-local-fork-cpu-v1'
    output.mkdir(exist_ok=False)
    mesh = np.load(source / 'mesh.npz')
    a, b = mesh['vertices_30'], mesh['vertices_31']
    fa, fb = mesh['faces_30'], mesh['faces_31']
    # These rings coincide geometrically, including angular samples. Remove
    # only their opposing caps and identify matching boundary vertices.
    ra, rb = np.arange(len(a) - 16, len(a)), np.arange(16)
    distance = np.linalg.norm(a[ra, None] - b[None, rb], axis=2)
    nearest = distance.argmin(axis=0)
    assert len(set(nearest.tolist())) == 16
    error = float(distance[nearest, rb].max())
    assert error < 1e-9
    keep_a = ~np.isin(fa, ra).all(axis=1)
    keep_b = ~np.isin(fb, rb).all(axis=1)
    assert int((~keep_a).sum()) == int((~keep_b).sum()) == 14
    mapping = np.concatenate([ra[nearest], np.arange(len(a), len(a) + len(b) - 16)])
    vertices = np.concatenate([a, b[16:]])
    faces = np.concatenate([fa[keep_a], mapping[fb[keep_b]]])
    check = audit(vertices, faces)
    assert not any(check[k] for k in ['nonmanifold_edges', 'winding_errors', 'zero_area'])
    assert check['volume'] > 0
    volume_before = audit(a, fa)['volume'] + audit(b, fb)['volume']
    assert abs(check['volume'] - volume_before) < 1e-7
    # Every retained body triangle stays in place; only duplicate ring vertex
    # coordinates are identified, at sub-nanometer floating-point tolerance.
    body_displacement = float(np.linalg.norm(vertices[mapping[fb[keep_b]]] - b[fb[keep_b]], axis=2).max())
    assert body_displacement < 1e-9
    arrays = dict(continuation_vertices=vertices, continuation_faces=faces)
    for i in [29, 93, 33, 96]:
        arrays[f'vertices_{i}'] = mesh[f'vertices_{i}']
        arrays[f'faces_{i}'] = mesh[f'faces_{i}']
    np.savez_compressed(output / 'minimal-forks.npz', **arrays)
    report = dict(status='CPU shared-ring continuation PASS; adjacent true forks still unresolved',
                  source_sha256=hashlib.sha256((source / 'mesh.npz').read_bytes()).hexdigest(),
                  exact_touch=dict(sections=[30, 31], traces=[140, 129], native=[488, 194],
                                   maximum_ring_error=error, removed_cap_triangles=28,
                                   maximum_retained_body_displacement=body_displacement,
                                   topology=check, volume_difference=check['volume'] - volume_before),
                  adjacent_forks=[dict(native=[488, 197], sections=[29, 30, 93]),
                                  dict(native=[488, 183], sections=[31, 33, 96])],
                  limitation='This proves an exact touching continuation can be welded without union. It does not identify the minimal Boolean failure or validate the adjacent forks.',
                  active_jobs=[], blender_lane='released; no launch authorized')
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
