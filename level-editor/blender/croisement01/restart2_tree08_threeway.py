"""Subtract neighboring volumes on each original triangle's own plane."""
import argparse
import faulthandler
import json
import subprocess
from pathlib import Path

import numpy as np

from restart2_tree08_cap_plan import read_paths, section_loops, serialize, triangulate
from restart2_tree08_local_fork import audit


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--fiveway', action='store_true')
    parser.add_argument('--revision', type=int, default=1)
    args = parser.parse_args()
    faulthandler.dump_traceback_later(30, repeat=True)
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    packet = root / 'tree08-v12-local-fork-cpu-v1'
    output = root / (f'tree08-v12-fiveway-v{args.revision}' if args.fiveway else f'tree08-v12-threeway-v{args.revision}')
    output.mkdir(exist_ok=True)
    assert not list(output.iterdir()), 'Refuse to overwrite an existing diagnostic'
    data = np.load(packet / 'minimal-forks.npz')
    origin = np.array([488., -667., 230.])
    meshes = [(data['continuation_vertices'] - origin, data['continuation_faces'])]
    ids = [29, 93, 33, 96] if args.fiveway else [29, 93]
    meshes += [(data[f'vertices_{i}'] - origin, data[f'faces_{i}']) for i in ids]
    bounds = [(v.min(0), v.max(0)) for v, _ in meshes]
    surfaces, counts = [], []
    for index, (vertices, faces) in enumerate(meshes):
        unchanged, rebuilt = 0, 0
        for face_index, face in enumerate(faces):
            if face_index % 200 == 0:
                print('SECTION/FACE', index, face_index, flush=True)
            triangle = vertices[face]
            low, high = triangle.min(0), triangle.max(0)
            neighbors = [j for j, (lo, hi) in enumerate(bounds) if j != index and not (np.any(high < lo) or np.any(low > hi))]
            if not neighbors:
                surfaces.append(triangle)
                unchanged += 1
                continue
            center = triangle.mean(0)
            normal = np.cross(triangle[1] - triangle[0], triangle[2] - triangle[0])
            normal /= np.linalg.norm(normal)
            u = triangle[0] - center
            u /= np.linalg.norm(u)
            basis = np.array([u, np.cross(normal, u)])
            loops = []
            for j in neighbors:
                loops.extend(section_loops(*meshes[j], center, normal, basis))
            if not loops:
                surfaces.append(triangle)
                unchanged += 1
                continue
            result = subprocess.run([str(packet / 'cap-clip')], input=serialize([(triangle - center) @ basis.T]) + serialize(loops), text=True, check=True, capture_output=True, timeout=10)
            patches = read_paths(iter(result.stdout.split()))
            try:
                triangles = triangulate(patches) if patches else []
            except AssertionError:
                failure = dict(status='HOLD: planar triangulation failed', section=index, face=face_index,
                               subject=((triangle-center) @ basis.T).tolist(),
                               clips=[p.tolist() for p in loops], patches=[p.tolist() for p in patches],
                               origin=(origin+center).tolist(), basis=basis.tolist())
                (output / 'failed-polygon.json').write_text(json.dumps(failure, indent=2) + '\n')
                raise
            for patch in triangles:
                world = center + patch @ basis
                if np.dot(np.cross(world[1] - world[0], world[2] - world[0]), normal) < 0:
                    world = world[::-1]
                surfaces.append(world)
            rebuilt += 1
        counts.append(dict(section=index, untouched_triangles=unchanged, reconstructed_triangles=rebuilt))
        print(counts[-1], flush=True)
    vertices, faces, lookup = [], [], {}
    maximum_weld = 0.
    for triangle in surfaces:
        face = []
        for point in triangle:
            key = tuple(np.round(point, 6))
            if key not in lookup:
                lookup[key] = len(vertices)
                vertices.append(point)
            vertex = lookup[key]
            maximum_weld = max(maximum_weld, float(np.linalg.norm(vertices[vertex] - point)))
            face.append(vertex)
        if len(set(face)) == 3:
            faces.append(face)
    vertices, faces = np.asarray(vertices), np.asarray(faces)
    np.savez_compressed(output / 'candidate.npz', vertices=vertices + origin, faces=faces)
    report = dict(status='CPU diagnostic; edge conformity and all geometry/source guards pending',
                  source_sections=[30, 31] + ids, construction=counts,
                  maximum_weld_displacement=maximum_weld, topology=audit(vertices, faces))
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    faulthandler.cancel_dump_traceback_later()


if __name__ == '__main__':
    main()
