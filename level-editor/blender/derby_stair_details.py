"""Add projected tread geometry to audited Derby stairs, retaining their supports."""
import json
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector


def add_steps(source, face_ids, count):
    name = source.name + " / modeled treads"
    if name in bpy.data.objects:
        raise RuntimeError(f"Already refined: {name}")
    faces = [source.data.polygons[i] for i in face_ids]
    indices = sorted({i for face in faces for i in face.vertices})
    if len(indices) != 4:
        raise ValueError("This stair helper requires a four-corner ramp")
    points = sorted((source.matrix_world @ source.data.vertices[i].co for i in indices), key=lambda p: p.z)
    low = sorted(points[:2], key=lambda p: p.x)
    high = points[2:]
    if (high[0] - low[0]).length_squared + (high[1] - low[1]).length_squared > (high[1] - low[0]).length_squared + (high[0] - low[1]).length_squared:
        high.reverse()
    base = min(p.z for p in low)
    rise = (sum(p.z for p in high) / 2 - base) / count
    if rise <= 0:
        raise ValueError("Stair must ascend")
    profile = [(0, base)]
    for i in range(count):
        profile.extend([(i / count, base + (i + 1) * rise), ((i + 1) / count, base + (i + 1) * rise)])
    profile.append((1, base))
    vertices = []
    for start, end in zip(low, high):
        for t, z in profile:
            p = start.lerp(end, t)
            p.z = z
            vertices.append(p)
    n = len(profile)
    polygons = [tuple(range(n)), tuple(range(n, 2 * n))]
    polygons.extend((i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n))
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], polygons)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not edge.is_manifold for edge in bm.edges)
    bad_faces = sum(face.calc_area() < 1e-8 for face in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad_edges or bad_faces:
        raise RuntimeError(f"Invalid generated staircase: {bad_edges} open edges, {bad_faces} degenerate faces")
    sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))

    def project(p):
        return Vector((p.x, -p.y * sin - p.z * cos, 1))

    triangle = faces[0]
    inverse = Matrix([project(source.matrix_world @ source.data.vertices[i].co) for i in triangle.vertices]).transposed().inverted()
    uvs = [source.data.uv_layers.active.data[i].uv.copy() for i in triangle.loop_indices]
    layer = mesh.uv_layers.new(name="Projected stair atlas")
    for loop in mesh.loops:
        weights = inverse @ project(mesh.vertices[loop.vertex_index].co)
        layer.data[loop.index].uv = sum((uvs[i] * weights[i] for i in range(3)), Vector((0, 0)))
    for material in source.data.materials:
        mesh.materials.append(material)
    obj = bpy.data.objects.new(name, mesh)
    bpy.data.collections["Derby Working"].objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in ("source_obstacle", "source_node", "asset_group", "asset_name", "part_name"):
        obj[key] = source[key]
    obj["step_count"] = count
    obj["todo"] = "Fine-fit riser spacing to the painted stair; concealed side texturing remains projected."
    return {"object": name, "steps": count, "nonmanifold_edges": bad_edges, "degenerate_faces": bad_faces}


def refine():
    working = bpy.data.collections["Derby Working"]
    report = []
    for index, faces, count in (
        (40, (8, 9), 6), (77, (8, 9), 12), (134, (6, 7), 10),
        (147, (6, 7), 9), (196, (8, 9), 10), (216, (8, 9), 3),
        (265, (6, 7), 12), (78, (6, 7), 18), (81, (6, 7), 16),
        (156, (9, 10), 7),
    ):
        source = next(o for o in working.objects if o.get("source_obstacle") == f"building-{index:03}" and not o.get("step_count"))
        report.append(add_steps(source, faces, count))
    bpy.context.view_layer.update()
    out = Path(bpy.data.filepath).parent
    (out / "stair-detail-validation.json").write_text(json.dumps(report, indent=2) + "\n")
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    return report
