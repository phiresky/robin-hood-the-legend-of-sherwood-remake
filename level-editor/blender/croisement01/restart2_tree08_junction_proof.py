"""Check a local source-surface junction before allowing broader construction."""
import argparse
import json
from pathlib import Path

import numpy as np
from vtkmodules.vtkCommonCore import vtkIdList
from vtkmodules.vtkCommonDataModel import vtkStaticCellLocator, vtkTriangle

from restart2_tree08_fork_kernel import poly


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
    args = parser.parse_args()
    suffix = '' if args.section == 29 else f'-{args.section}'
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    packet = root / f'tree08-v12-planar-junction{suffix}-stitched-v1'
    mesh = np.load(packet / 'candidate.npz')
    vertices, faces = mesh['vertices'], mesh['faces']
    triangles = vertices[faces]
    lower, upper = triangles.min(1), triangles.max(1)
    locator = vtkStaticCellLocator()
    locator.SetDataSet(poly(vertices, faces))
    locator.BuildLocator()
    failures, counts = [], dict(nonadjacent=0, adjacent_interior=0)
    centers = triangles.mean(1)
    # Inset is only for shared-index pairs, where boundary contact is required.
    # It does not replace exact nonadjacent intersection testing.
    inset = centers[:, None] + (triangles - centers[:, None]) * (1 - 1e-7)
    for i, triangle in enumerate(triangles):
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
                failures.append(dict(faces=[i, j], kind=key))
    source = np.load(root / 'tree08-v12-local-fork-cpu-v1/minimal-forks.npz')
    before = native_depth([(source['continuation_vertices'], source['continuation_faces']), (source[f'vertices_{args.section}'], source[f'faces_{args.section}'])])
    after = native_depth([(vertices, faces)])
    common = np.isfinite(before) & np.isfinite(after)
    lost = int((np.isfinite(before) & ~np.isfinite(after)).sum())
    gained = int((~np.isfinite(before) & np.isfinite(after)).sum())
    depth_error = float(np.abs(before[common] - after[common]).max())
    report = dict(status='FAIL' if failures or lost or gained or depth_error > .0002 else 'PASS_LOCAL_DIAGNOSTICS', tested_intersections=counts,
                  intersections=failures, native_pixels=int(common.sum()),
                  native_lost=lost, native_gained=gained,
                  maximum_native_front_depth_difference=depth_error,
                  adjacent_test_inset_fraction=1e-7,
                  scope='Single isolated fork only. Shared-boundary test cannot resolve overlap below the explicit inset tolerance; full-tree topology/material/contact review remains required.')
    (packet / 'source-and-intersections-planar.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({**report, 'intersections': failures[:12], 'intersection_count': len(failures)}, indent=2))


if __name__ == '__main__':
    main()
