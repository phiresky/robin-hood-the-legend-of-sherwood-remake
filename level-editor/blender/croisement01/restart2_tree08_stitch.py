"""Conform planar junction edges without changing their geometric surface."""
import argparse
import json
from collections import Counter
from pathlib import Path

import numpy as np
from scipy.spatial import cKDTree

from restart2_tree08_local_fork import audit


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--section', type=int, choices=[29, 93, 33, 96], default=29)
    parser.add_argument('--threeway', action='store_true')
    parser.add_argument('--fiveway', action='store_true')
    parser.add_argument('--revision', type=int, default=1)
    parser.add_argument('--group', type=int, choices=[0, 1])
    args = parser.parse_args()
    suffix = '' if args.section == 29 else f'-{args.section}'
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    source = root / f'tree08-v12-planar-junction{suffix}-v1'
    output = root / f'tree08-v12-planar-junction{suffix}-stitched-v1'
    if args.threeway:
        source = root / 'tree08-v12-threeway-v1'
        output = root / 'tree08-v12-threeway-stitched-v1'
    if args.fiveway:
        source = root / f'tree08-v12-fiveway-v{args.revision}'
        output = root / f'tree08-v12-fiveway-stitched-v{args.revision}'
    if args.group is not None:
        source = root / f'tree08-v12-remaining-group{args.group}-v{args.revision}'
        output = root / f'tree08-v12-remaining-group{args.group}-stitched-v{args.revision}'
    output.mkdir(exist_ok=False)
    merge_report = dict(maximum_identification_displacement=0., collapsed_triangles=0)
    if args.group is None:
        mesh = np.load(source / 'candidate.npz')
        vertices, faces = mesh['vertices'], mesh['faces']
    else:
        sections = json.loads((source / 'scope.json').read_text())['sections']
        triangles = np.concatenate([np.load(source / f'part-{i}.npz')['triangles'] for i in sections])
        unique, inverse = np.unique(triangles.reshape(-1, 3), axis=0, return_inverse=True)
        parents = list(range(len(unique)))
        def find(index):
            while parents[index] != index:
                parents[index] = parents[parents[index]]
                index = parents[index]
            return index
        for a, b in cKDTree(unique).query_pairs(1e-6, output_type='ndarray'):
            a, b = find(int(a)), find(int(b))
            parents[max(a, b)] = min(a, b)
        roots = np.array([find(i) for i in range(len(unique))])
        displacement = float(np.linalg.norm(unique-unique[roots], axis=1).max())
        assert displacement <= 2e-6, 'Numerical identification exceeded its fixed local bound'
        used, remap = np.unique(roots, return_inverse=True)
        vertices, faces = unique[used], remap[inverse].reshape(-1, 3)
        keep = (faces[:, 0] != faces[:, 1]) & (faces[:, 1] != faces[:, 2]) & (faces[:, 2] != faces[:, 0])
        merge_report = dict(maximum_identification_displacement=displacement, collapsed_triangles=int((~keep).sum()))
        faces = faces[keep]
    edges = Counter(tuple(sorted((int(f[i]), int(f[(i + 1) % 3])))) for f in faces for i in range(3))
    tree = cKDTree(vertices)
    splits = {}
    max_distance = 0.
    for (a, b), count in edges.items():
        if count == 2:
            continue
        start, end = vertices[[a, b]]
        direction = end - start
        length = np.linalg.norm(direction)
        found = []
        for index in tree.query_ball_point((start + end) / 2, length / 2 + 1e-7):
            if index in [a, b]:
                continue
            point = vertices[index]
            t = float(np.dot(point - start, direction) / length**2)
            distance = float(np.linalg.norm(point - start - t * direction))
            if 1e-9 < t < 1 - 1e-9 and distance < 1e-7:
                found.append((t, index))
                max_distance = max(max_distance, distance)
        if found:
            splits[a, b] = [index for _, index in sorted(found)]
    result_vertices = vertices.tolist()
    result_faces = []
    changed = 0
    for face in faces:
        boundary = []
        for a, b in zip(face, np.roll(face, -1)):
            boundary.append(int(a))
            boundary.extend(splits.get((int(a), int(b)), list(reversed(splits.get((int(b), int(a)), [])))))
        if len(boundary) == 3:
            result_faces.append(face.tolist())
            continue
        changed += 1
        center = len(result_vertices)
        result_vertices.append(vertices[face].mean(0).tolist())
        for a, b in zip(boundary, boundary[1:] + boundary[:1]):
            result_faces.append([a, b, center])
    result_vertices, result_faces = np.array(result_vertices), np.array(result_faces)
    np.savez_compressed(output / 'candidate.npz', vertices=result_vertices, faces=result_faces)
    report = dict(status='DIAGNOSTIC: topology checked; self-intersection/source guards still pending',
                  numerical_identification=merge_report,
                  split_edges=len(splits), subdivided_triangles=changed,
                  maximum_edge_insertion_distance=max_distance,
                  retained_vertex_displacement=float(np.linalg.norm(result_vertices[:len(vertices)] - vertices, axis=1).max()),
                  topology=audit(result_vertices, result_faces))
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
