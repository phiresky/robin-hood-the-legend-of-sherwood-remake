"""Build a small union by clipping original surfaces, without a 3D Boolean."""
import argparse
import json
import subprocess
from pathlib import Path

import numpy as np
from scipy.spatial import ConvexHull

from restart2_tree08_cap_plan import read_paths, section_loops, serialize, triangulate
from restart2_tree08_local_fork import audit


def clip(poly, normal, offset, keep_inside):
    distances = poly @ normal + offset
    if not keep_inside:
        distances = -distances
    result = []
    for a, b, da, db in zip(poly, np.roll(poly, -1, axis=0), distances, np.roll(distances, -1)):
        if da <= 1e-10:
            result.append(a)
        if (da < -1e-10 and db > 1e-10) or (da > 1e-10 and db < -1e-10):
            result.append(a + (b - a) * da / (da - db))
    return np.asarray(result).reshape(-1, 3)


def fan(poly):
    return [np.array([poly[0], poly[i], poly[i + 1]]) for i in range(1, len(poly) - 1)
            if np.linalg.norm(np.cross(poly[i] - poly[0], poly[i + 1] - poly[0])) > 1e-9]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--section', type=int, choices=[29, 93, 33, 96], default=29)
    args = parser.parse_args()
    suffix = '' if args.section == 29 else f'-{args.section}'
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    packet = root / 'tree08-v12-local-fork-cpu-v1'
    output = root / f'tree08-v12-planar-junction{suffix}-v1'
    output.mkdir(exist_ok=False)
    data = np.load(packet / 'minimal-forks.npz')
    origin = np.array([488., -667., 230.])
    parent = data['continuation_vertices'] - origin
    child, child_faces = data[f'vertices_{args.section}'] - origin, data[f'faces_{args.section}']
    hull = ConvexHull(parent)
    planes = {}
    for plane in hull.equations:
        planes.setdefault(tuple(np.round(plane, 8)), plane)
    surfaces = []
    # A convex parent permits exact exterior subdivision of each child triangle.
    # Each output polygon lies on the original triangle, with no smoothing.
    for triangle in child[child_faces]:
        interior = triangle
        for equation in planes.values():
            if len(interior) < 3:
                break
            outside = clip(interior, equation[:3], equation[3], False)
            surfaces.extend(fan(outside))
            interior = clip(interior, equation[:3], equation[3], True)
    child_output_triangles = len(surfaces)
    facets = []
    for number, equation in enumerate(planes.values()):
        normal, offset = equation[:3], equation[3]
        on_plane = parent[np.abs(parent @ normal + offset) < 1e-7]
        center = on_plane.mean(0)
        u = on_plane[0] - center
        u /= np.linalg.norm(u)
        basis = np.array([u, np.cross(normal, u)])
        projected = (on_plane - center) @ basis.T
        polygon = projected[ConvexHull(projected).vertices]
        loops = section_loops(child, child_faces, center, normal, basis)
        if loops:
            result = subprocess.run([str(packet / 'cap-clip')], input=serialize([polygon]) + serialize(loops), text=True, check=True, capture_output=True)
            patches = read_paths(iter(result.stdout.split()))
            triangles = triangulate(patches) if patches else []
        else:
            triangles = [polygon[[0, i, i + 1]] for i in range(1, len(polygon) - 1)]
        for triangle in triangles:
            world = center + triangle @ basis
            if np.dot(np.cross(world[1] - world[0], world[2] - world[0]), normal) < 0:
                world = world[::-1]
            surfaces.append(world)
        facets.append(dict(facet=number, section_loops=len(loops), output_triangles=len(triangles)))
    # This first pass deliberately does not patch open edges. Its topology audit
    # exposes any unmatched subdivision requiring shared intersection vertices.
    vertices, faces, lookup = [], [], {}
    max_weld = 0.
    for triangle in surfaces:
        face = []
        for point in triangle:
            key = tuple(np.round(point, 6))
            if key not in lookup:
                lookup[key] = len(vertices)
                vertices.append(point)
            index = lookup[key]
            max_weld = max(max_weld, float(np.linalg.norm(vertices[index] - point)))
            face.append(index)
        if len(set(face)) == 3:
            faces.append(face)
    vertices, faces = np.array(vertices), np.array(faces)
    np.savez_compressed(output / 'candidate.npz', vertices=vertices + origin, faces=faces)
    report = dict(status='DIAGNOSTIC: shared subdivision and source proof pending',
                  authority='Original section surfaces only; child triangles clipped against exact convex continuation halfspaces',
                  convex_parent_facets=len(planes), child_output_triangles=child_output_triangles,
                  maximum_vertex_weld_displacement=max_weld, topology=audit(vertices, faces), facets=facets)
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'facets'}, indent=2))


if __name__ == '__main__':
    main()
