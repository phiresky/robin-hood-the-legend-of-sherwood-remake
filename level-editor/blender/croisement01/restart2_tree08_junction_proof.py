"""Check a local source-surface junction before allowing broader construction."""
import argparse
import json
from decimal import Decimal, localcontext
from pathlib import Path

import numpy as np
from vtkmodules.vtkCommonCore import vtkIdList
from vtkmodules.vtkCommonDataModel import vtkStaticCellLocator, vtkTriangle

from restart2_tree08_fork_kernel import poly
from restart2_tree08_local_fork import audit


def intersection(a, b):
    normal = np.cross(a[1] - a[0], a[2] - a[0])
    normal /= np.linalg.norm(normal)
    other = np.cross(b[1] - b[0], b[2] - b[0])
    other /= np.linalg.norm(other)
    for distances in [(b - a[0]) @ normal, (a - b[0]) @ other]:
        if distances.min() > 1e-14 or distances.max() < -1e-14:
            return False
    if np.linalg.norm(np.cross(normal, other)) < 1e-8 and np.abs((b - a[0]) @ normal).max() < 1e-8:
        # VTK's 3D predicate is unstable for almost-coplanar shared edges.
        # In their own plane, separating axes test all six exact triangle edges.
        axes = [i for i in range(3) if i != int(np.argmax(np.abs(normal)))]
        x, y = (a - a[0])[:, axes], (b - a[0])[:, axes]
        for triangle in [x, y]:
            for edge in np.roll(triangle, -1, axis=0) - triangle:
                direction = np.array([-edge[1], edge[0]])
                direction /= np.linalg.norm(direction)
                p, q = x @ direction, y @ direction
                if min(p.max(), q.max()) - max(p.min(), q.min()) <= 1e-12:
                    return False
        return True
    return bool(vtkTriangle.TrianglesIntersect(*a, *b))



def precise_plane_separation(first, second, adjacent):
    """Confirm strict separation using stored coordinates, without epsilon changes."""
    with localcontext() as context:
        context.prec = 80
        triangles = []
        for source in [first, second]:
            triangle = [[Decimal(float(value)) for value in row] for row in source]
            if adjacent:
                center = [sum(row[j] for row in triangle) / 3 for j in range(3)]
                triangle = [[center[j] + (row[j]-center[j])*Decimal('0.9999999')
                             for j in range(3)] for row in triangle]
            triangles.append(triangle)
        def determinant(a, b, c, d):
            x, y, z = [[point[j]-a[j] for j in range(3)] for point in [b, c, d]]
            return (x[0]*(y[1]*z[2]-y[2]*z[1])
                    - x[1]*(y[0]*z[2]-y[2]*z[0])
                    + x[2]*(y[0]*z[1]-y[1]*z[0]))
        for a, b in [triangles, triangles[::-1]]:
            signs = [determinant(*a, point) for point in b]
            if all(value > 0 for value in signs) or all(value < 0 for value in signs):
                return True
        def subtract(a, b): return [x-y for x, y in zip(a, b)]
        def cross(a, b):
            return [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]]
        a, b = triangles
        normals = [cross(subtract(t[1], t[0]), subtract(t[2], t[0])) for t in triangles]
        direction = cross(*normals)
        if any(direction):
            axis = max(range(3), key=lambda index: abs(direction[index]))
            intervals = []
            for triangle, plane in [(a, b), (b, a)]:
                distances = [determinant(*plane, point) for point in triangle]
                points = [point[axis] for point, distance in zip(triangle, distances) if distance == 0]
                for i in range(3):
                    j = (i+1) % 3
                    if distances[i]*distances[j] < 0:
                        fraction = distances[i]/(distances[i]-distances[j])
                        points.append(triangle[i][axis]+fraction*(triangle[j][axis]-triangle[i][axis]))
                if not points:
                    return True
                intervals.append((min(points), max(points)))
            # The two nonparallel planes intersect in one line. Disjoint
            # clipped intervals on any nonconstant line coordinate cannot meet.
            if min(high for low, high in intervals) < max(low for low, high in intervals):
                return True
    return False


def native_depth(sections):
    result = np.full((461, 446), -np.inf)
    sine, cosine = np.sin(np.radians(35)), np.cos(np.radians(35))
    for vertices, faces in sections:
        xy = np.column_stack((vertices[:, 0] - 331, -vertices[:, 1] * sine - vertices[:, 2] * cosine - 11))
        depth = -vertices[:, 1] * cosine + vertices[:, 2] * sine
        for face in faces:
            a, b, c = xy[face]
            lower = np.maximum(0, np.ceil(np.minimum(np.minimum(a, b), c) - .5).astype(int))
            upper = np.minimum([445, 460], np.floor(np.maximum(np.maximum(a, b), c) - .5).astype(int))
            if np.any(lower > upper):
                continue
            determinant = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
            if abs(determinant) < 1e-10:
                continue
            yy, xx = np.mgrid[lower[1]:upper[1] + 1, lower[0]:upper[0] + 1]
            x, y = xx + .5 - a[0], yy + .5 - a[1]
            u = (x * (c[1] - a[1]) - y * (c[0] - a[0])) / determinant
            v = ((b[0] - a[0]) * y - (b[1] - a[1]) * x) / determinant
            inside = (u >= -1e-8) & (v >= -1e-8) & (u + v <= 1 + 1e-8)
            value = depth[face[0]] + u * (depth[face[1]] - depth[face[0]]) + v * (depth[face[2]] - depth[face[0]])
            result[yy, xx] = np.maximum(result[yy, xx], np.where(inside, value, -np.inf))
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--section', type=int, choices=[29, 93, 33, 96], default=29)
    parser.add_argument('--threeway', action='store_true')
    parser.add_argument('--fiveway', action='store_true')
    parser.add_argument('--revision', type=int, default=1)
    parser.add_argument('--group', type=int, choices=[0, 1])
    parser.add_argument('--conformed', action='store_true')
    args = parser.parse_args()
    suffix = '' if args.section == 29 else f'-{args.section}'
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    packet = root / f'tree08-v12-planar-junction{suffix}-stitched-v1'
    if args.threeway:
        packet = root / 'tree08-v12-threeway-stitched-v1'
    if args.fiveway:
        packet = root / f'tree08-v12-fiveway-stitched-v{args.revision}'
    if args.group is not None:
        packet = root / f'tree08-v12-remaining-group{args.group}-stitched-v{args.revision}'
    if args.conformed:
        packet = packet.with_name(packet.name + '-conformed')
    mesh = np.load(packet / 'candidate.npz')
    vertices, faces = mesh['vertices'], mesh['faces']
    topology = audit(vertices, faces)
    parents = list(range(len(vertices)))
    def find(index):
        while parents[index] != index:
            parents[index] = parents[parents[index]]
            index = parents[index]
        return index
    for a, b, c in faces:
        parents[find(int(b))] = find(int(a))
        parents[find(int(c))] = find(int(a))
    components = len({find(i) for i in range(len(vertices))})
    topology['components'] = components
    triangles = vertices[faces]
    lower, upper = triangles.min(1), triangles.max(1)
    locator = vtkStaticCellLocator()
    locator.SetDataSet(poly(vertices, faces))
    locator.BuildLocator()
    failures, counts = [], dict(nonadjacent=0, adjacent_interior=0)
    precise_separations = []
    centers = triangles.mean(1)
    # Inset is only for shared-index pairs, where boundary contact is required.
    # It does not replace exact nonadjacent intersection testing.
    inset = centers[:, None] + (triangles - centers[:, None]) * (1 - 1e-7)
    for i, triangle in enumerate(triangles):
        if i % 2000 == 0:
            print('INTERSECTION FACE', i, '/', len(triangles), flush=True)
        lo, hi = lower[i], upper[i]
        candidates = vtkIdList()
        locator.FindCellsWithinBounds([lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]], candidates)
        for k in range(candidates.GetNumberOfIds()):
            j = candidates.GetId(k)
            if j <= i or np.any(upper[j] < lo - 1e-9) or np.any(lower[j] > hi + 1e-9):
                continue
            shared = set(faces[i]) & set(faces[j])
            adjacent = bool(shared)
            if len(shared) == 2:
                first_normal = np.cross(triangle[1] - triangle[0], triangle[2] - triangle[0])
                second_normal = np.cross(triangles[j, 1] - triangles[j, 0], triangles[j, 2] - triangles[j, 0])
                cross_angle = np.linalg.norm(np.cross(first_normal / np.linalg.norm(first_normal), second_normal / np.linalg.norm(second_normal)))
                # Distinct planes containing the same edge intersect only on
                # that edge. Coplanar neighbors still need the overlap test.
                if cross_angle > 1e-12:
                    continue
            key = 'adjacent_interior' if adjacent else 'nonadjacent'
            a, b = (inset[i], inset[j]) if adjacent else (triangle, triangles[j])
            if np.any(a.max(0) < b.min(0)) or np.any(b.max(0) < a.min(0)):
                continue
            counts[key] += 1
            if intersection(a, b):
                if precise_plane_separation(triangle, triangles[j], adjacent):
                    precise_separations.append(dict(faces=[i, j], kind=key))
                    continue
                failures.append(dict(faces=[i, j], kind=key))
    (packet / 'precise-plane-separations.json').write_text(json.dumps(dict(method='80-digit Decimal determinants and nonparallel-plane line intervals; original stored coordinates, existing shared-boundary inset only; strict separation, no tolerance relaxation', separated=precise_separations), indent=2) + '\n')
    source = np.load(root / 'tree08-v12-local-fork-cpu-v1/minimal-forks.npz')
    ids = [29, 93] if args.threeway else [args.section]
    if args.fiveway:
        ids = [29, 93, 33, 96]
    references = [(source['continuation_vertices'], source['continuation_faces'])] + [(source[f'vertices_{i}'], source[f'faces_{i}']) for i in ids]
    if args.group is not None:
        from restart2_tree08_remaining_forks import inputs
        meshes, _, groups, origin = inputs(root)
        references = [(meshes[i][0]+origin, meshes[i][1]) for i in groups[args.group]]
    before = native_depth(references)
    after = native_depth([(vertices, faces)])
    common = np.isfinite(before) & np.isfinite(after)
    lost = int((np.isfinite(before) & ~np.isfinite(after)).sum())
    gained = int((~np.isfinite(before) & np.isfinite(after)).sum())
    depth_error = float(np.abs(before[common] - after[common]).max())
    topology_fail = components != 1 or topology['volume'] <= 0 or any(topology[k] for k in ['nonmanifold_edges', 'winding_errors', 'zero_area'])
    report = dict(status='FAIL' if topology_fail or failures or lost or gained or depth_error > .0002 else 'PASS_LOCAL_DIAGNOSTICS', topology=topology, tested_intersections=counts,
                  intersections=failures, native_pixels=int(common.sum()),
                  native_lost=lost, native_gained=gained,
                  maximum_native_front_depth_difference=depth_error,
                  adjacent_test_inset_fraction=1e-7,
                  scope='Single isolated fork only. Shared-boundary test cannot resolve overlap below the explicit inset tolerance; full-tree topology/material/contact review remains required.')
    (packet / 'source-and-intersections-planar.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({**report, 'intersections': failures[:12], 'intersection_count': len(failures)}, indent=2))


if __name__ == '__main__':
    main()
