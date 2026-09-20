"""Audit timber without cutting logs hidden behind the foreground bucket.

The composite native mask includes both timber and bucket. Shortening logs to
eliminate projected bucket pixels would manufacture unsupported geometry.
"""
import math
import bpy
import bmesh
import numpy as np
from mathutils import Vector

ASSET = "derby-east-bailey-stacked-timber"


def connected_components(mesh):
    adjacent = [set() for _ in mesh.vertices]
    for edge in mesh.edges:
        a, b = edge.vertices
        adjacent[a].add(b)
        adjacent[b].add(a)
    unseen = set(range(len(adjacent)))
    groups = []
    while unseen:
        pending = [min(unseen)]
        unseen.remove(pending[0])
        group = []
        while pending:
            current = pending.pop()
            group.append(current)
            new = adjacent[current] & unseen
            unseen.difference_update(new)
            pending.extend(sorted(new))
        groups.append(sorted(group))
    return groups


def fit_axis(points):
    coordinates = np.array([tuple(point) for point in points], dtype=float)
    center = coordinates.mean(axis=0)
    _, vectors = np.linalg.eigh((coordinates-center).T @ (coordinates-center))
    axis = Vector(vectors[:, -1])
    if axis.x < 0:
        axis.negate()
    if abs(axis.z) > .001 or axis.x < .1:
        raise ValueError("Expected horizontal timber cylinder")
    return Vector(center), axis


def refine():
    """Return geometry evidence, leaving scene geometry and identities intact."""
    objects = [o for o in bpy.data.collections["Derby Working"].all_objects
               if o.type == "MESH" and not o.hide_render and o.get("asset_group") == ASSET]
    if len(objects) != 2 or {o.get("source_node") for o in objects} != {"building-107", "building-108"}:
        raise ValueError("Expected exactly the two owned timber meshes")
    report, cylinders = [], []
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    for obj in sorted(objects, key=lambda o: o["source_node"]):
        world = [obj.matrix_world @ v.co for v in obj.data.vertices]
        groups = connected_components(obj.data)
        if len(groups) != int(obj.get("log_count", -1)):
            raise ValueError("Connected components differ from recorded log count")
        ends = []
        for indices in groups:
            points = [world[index] for index in indices]
            center, axis = fit_axis(points)
            along = [(p-center).dot(axis) for p in points]
            radius = max((p-center-axis*(p-center).dot(axis)).length for p in points)
            cylinders.append((center, axis, radius))
            caps = [center+axis*t for t in (min(along), max(along))]
            ends.append({"source_caps": [[p.x, -p.y*sine-p.z*cosine] for p in caps],
                         "length": max(along)-min(along), "radius": radius})
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        defects = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                   "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
        bm.free()
        if any(defects.values()):
            raise ValueError(defects)
        report.append({"source_node": obj["source_node"], "logs": len(groups),
                       "caps": ends, "min_world_z": min(p.z for p in world), **defects})
    gaps = []
    for i, (center, axis, radius) in enumerate(cylinders):
        nearby = []
        for j, (other, other_axis, other_radius) in enumerate(cylinders):
            if i == j:
                continue
            if abs(axis.dot(other_axis)) < .999:
                raise ValueError("Unexpected nonparallel cylinders")
            delta = other-center
            nearby.append((delta-axis*delta.dot(axis)).length-radius-other_radius)
        gaps.append(min(nearby))
    return {"status": "reviewed_no_geometry_change", "parts": report,
            "nearest_neighbor_gap_range": [min(gaps), max(gaps)],
            "rejected_change": "Source-x truncation removes potentially valid logs behind bucket",
            "remaining": ["Foreground bucket needs separate geometry/ownership review",
                          "Composite mask 30 is not an exclusive timber mask",
                          "Exact exposed end correspondence and hidden count remain uncertain"]}
