"""Offline physical candidate with sampled, bidirectional deviation reporting.

The caller must still check closed topology and solid decomposition. Sampling
does not certify a maximum surface/contact error. Visual asset files are unused.
"""
import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree


def sample_distance(vertices, faces, target):
    samples = 0
    maximum = 0.0

    def visit(point):
        nonlocal samples, maximum
        nearest = target.find_nearest(Vector(point))
        if nearest[0] is None or not math.isfinite(nearest[3]):
            raise ValueError("Physical deviation query did not find a finite surface")
        samples += 1
        maximum = max(maximum, nearest[3])

    for point in vertices:
        visit(point)
    edges = set()
    for face in faces:
        points = [vertices[i] for i in face]
        visit([sum(p[axis] for p in points) / 3 for axis in range(3)])
        for i in range(3):
            edge = tuple(sorted((face[i], face[(i + 1) % 3])))
            if edge not in edges:
                edges.add(edge)
                visit([(vertices[edge[0]][axis] + vertices[edge[1]][axis]) / 2
                       for axis in range(3)])
    return {"samples": samples, "maximumDistance": maximum}


def main():
    input_path, output_path, ratio_text = sys.argv[sys.argv.index("--") + 1:]
    ratio = float(ratio_text)
    if not math.isfinite(ratio) or not 0 < ratio <= 1:
        raise ValueError("Physical decimation ratio must be in (0, 1]")
    source = json.loads(Path(input_path).read_text())
    vertices, faces = source["vertices"], source["faces"]
    mesh = bpy.data.meshes.new("physical-candidate")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new("physical-candidate", mesh)
    bpy.context.collection.objects.link(obj)
    modifier = obj.modifiers.new("physical-decimation", "DECIMATE")
    modifier.ratio = ratio
    modifier.use_collapse_triangulate = True
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    result = evaluated.to_mesh()
    result.calc_loop_triangles()
    output_vertices = [list(v.co) for v in result.vertices]
    output_faces = [list(t.vertices) for t in result.loop_triangles]
    if not output_faces:
        raise ValueError("Physical decimation erased the shell")
    original_bvh = BVHTree.FromPolygons(vertices, faces, all_triangles=True)
    candidate_bvh = BVHTree.FromPolygons(output_vertices, output_faces, all_triangles=True)
    report = {
        "method": "blender-collapse",
        "blenderVersion": bpy.app.version_string,
        "requestedRatio": ratio,
        "sourceTriangles": len(faces),
        "triangles": len(output_faces),
        "sourceToCandidate": sample_distance(vertices, faces, candidate_bvh),
        "candidateToSource": sample_distance(output_vertices, output_faces, original_bvh),
        "scope": "sampled-deviation-not-certified-contact-bound",
    }
    Path(output_path).write_text(json.dumps({
        "vertices": output_vertices, "faces": output_faces, "report": report,
    }))
    print(json.dumps(report))


if __name__ == "__main__":
    main()
