"""Reconstruct exposed cap regions from untouched source-section intersections."""
import json
import subprocess
from collections import defaultdict
from pathlib import Path

import numpy as np
from vtkmodules.vtkCommonCore import vtkPoints
from vtkmodules.vtkCommonDataModel import vtkCellArray, vtkPolyData
from vtkmodules.vtkFiltersGeneral import vtkContourTriangulator
from vtkmodules.util.numpy_support import vtk_to_numpy


def section_loops(vertices, faces, origin, normal, basis):
    segments = set()
    points = {}
    for triangle in vertices[faces]:
        distances = (triangle - origin) @ normal
        if distances.min() > 1e-9 or distances.max() < -1e-9:
            continue
        hits = []
        for a, b, da, db in zip(triangle, np.roll(triangle, -1, axis=0), distances, np.roll(distances, -1)):
            if abs(da) < 1e-9:
                hits.append(a)
            if da * db < 0:
                hits.append(a + (b - a) * da / (da - db))
        keys = []
        for hit in hits:
            uv = (hit - origin) @ basis.T
            key = tuple(np.round(uv, 7))
            points[key] = uv
            if key not in keys:
                keys.append(key)
        if len(keys) == 2:
            segments.add(tuple(sorted(keys)))
        elif len(keys) > 2:
            raise RuntimeError('Coplanar triangle requires explicit handling')
    adjacency = defaultdict(set)
    for a, b in segments:
        adjacency[a].add(b)
        adjacency[b].add(a)
    assert all(len(neighbors) == 2 for neighbors in adjacency.values()), 'Open or branched section'
    remaining = set(adjacency)
    loops = []
    while remaining:
        start = min(remaining)
        current, previous = start, None
        loop = []
        while True:
            loop.append(points[current])
            remaining.remove(current)
            following = sorted(adjacency[current] - {previous})[0]
            previous, current = current, following
            if current == start:
                break
        loops.append(np.array(loop))
    return loops


def serialize(paths):
    lines = [str(len(paths))]
    for path in paths:
        lines.append(str(len(path)) + ' ' + ' '.join(format(float(x), '.17g') for x in np.asarray(path).flat))
    return '\n'.join(lines) + '\n'


def read_paths(tokens):
    paths = []
    for _ in range(int(next(tokens))):
        count = int(next(tokens))
        paths.append(np.array([[float(next(tokens)), float(next(tokens))] for _ in range(count)]))
    return paths


def area(poly):
    return float(np.sum(poly[:, 0] * np.roll(poly[:, 1], -1) - poly[:, 1] * np.roll(poly[:, 0], -1)) / 2)


def triangulate(paths):
    points, lines = vtkPoints(), vtkCellArray()
    points.SetDataTypeToDouble()
    for path in paths:
        indices = [points.InsertNextPoint(float(p[0]), float(p[1]), 0.) for p in path]
        for a, b in zip(indices, indices[1:] + indices[:1]):
            lines.InsertNextCell(2)
            lines.InsertCellPoint(a)
            lines.InsertCellPoint(b)
    data = vtkPolyData()
    data.SetPoints(points)
    data.SetLines(lines)
    operation = vtkContourTriangulator()
    operation.SetInputData(data)
    operation.Update()
    result = operation.GetOutput()
    assert operation.GetTriangulationError() == 0
    vertices = vtk_to_numpy(result.GetPoints().GetData())[:, :2]
    faces = vtk_to_numpy(result.GetPolys().GetConnectivityArray()).reshape(-1, 3)
    return vertices[faces]


def main():
    root = Path(__file__).resolve().parents[2] / 'work/croisement01-refinement/restart2'
    packet = root / 'tree08-v12-local-fork-cpu-v1'
    mesh = np.load(packet / 'minimal-forks.npz')
    output = root / 'tree08-v12-original-cap-plan-v2'
    output.mkdir(exist_ok=True)
    assert not list(output.iterdir()), 'Refuse to replace existing diagnostic output'
    records = []
    for index, end in [(29, 0), (93, 0), (33, -1), (96, -1)]:
        ring = mesh['continuation_vertices'].reshape(-1, 16, 3)[end]
        origin = ring.mean(0)
        u = ring[0] - origin
        u /= np.linalg.norm(u)
        normal = np.cross(ring[1] - ring[0], ring[2] - ring[0])
        normal /= np.linalg.norm(normal)
        basis = np.array([u, np.cross(normal, u)])
        subject = (ring - origin) @ basis.T
        loops = section_loops(mesh[f'vertices_{index}'], mesh[f'faces_{index}'], origin, normal, basis)
        result = subprocess.run([str(packet / 'cap-clip')], input=serialize([subject]) + serialize(loops), text=True, check=True, capture_output=True)
        tokens = iter(result.stdout.split())
        remainder = read_paths(tokens)
        triangles = triangulate(remainder)
        tri_area = sum(abs(area(t)) for t in triangles)
        polygon_area = abs(sum(area(p) for p in remainder))
        assert abs(tri_area - polygon_area) < 1e-6
        world_triangles = np.array([origin + t @ basis for t in triangles])
        assert np.abs((world_triangles - origin) @ normal).max() < 1e-10
        np.savez_compressed(output / f'cap-{index}.npz', triangles=world_triangles,
                            origin=origin, basis=basis, cross_section=np.concatenate(loops))
        records.append(dict(section=index, endpoint=end, cross_section_loops=len(loops),
                            cross_section_vertices=sum(map(len, loops)), triangles=len(triangles),
                            source_cap_area=abs(area(subject)), exposed_cap_area=tri_area,
                            planar_area_error=abs(tri_area - polygon_area)))
    report = dict(status='Original cap planar subtraction/triangulation PASS; stitching and whole-junction proof pending',
                  kernel='Installed Boost.Geometry difference plus planar contour triangulation, double local coordinates',
                  input_authority='Frozen original section meshes; no VTK output reused', caps=records,
                  limitation='These are isolated planar patches. Side strips require matching intersection vertices and manifold/self-intersection/source tests before any acceptance.')
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
