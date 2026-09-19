"""First Derby detail pass: a reversible staircase replacement with projected UVs."""
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector, Matrix


def refine_stairs(replace_existing=False):
    name = "Derby Lower Courtyard Steps"
    collection = bpy.data.collections["Derby Working"]
    existing = next((o for o in collection.objects if o.get("step_count") and o.get("source_obstacle") == "building-012"), None)
    if existing and not replace_existing:
        raise RuntimeError("Stair pass already exists")
    source = next(o for o in collection.objects if o.get("source_obstacle") == "building-012" and not o.get("step_count"))
    # Audited top surface corners, ordered lower left/right, upper left/right.
    points = [source.matrix_world @ source.data.vertices[i].co for i in (14, 15, 13, 12)]
    lower_left, lower_right, upper_left, upper_right = points
    count = 20
    sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))

    def project(p):
        return Vector((p.x, -p.y * sin - p.z * cos, 1))

    face = source.data.polygons[6]
    corners = [source.matrix_world @ source.data.vertices[i].co for i in face.vertices]
    uv = [source.data.uv_layers.active.data[i].uv.copy() for i in face.loop_indices]
    inverse = Matrix([project(p) for p in corners]).transposed().inverted()
    base = min(lower_left.z, lower_right.z)
    rise = ((upper_left.z + upper_right.z) / 2 - base) / count
    profile = [(0, base)]
    for i in range(count):
        profile.extend([(i / count, base + rise * (i + 1)), ((i + 1) / count, base + rise * (i + 1))])
    profile.append((1, base))
    vertices = []
    for start, end in ((lower_left, upper_left), (lower_right, upper_right)):
        for t, z in profile:
            p = start.lerp(end, t)
            p.z = z
            vertices.append(p)
    n = len(profile)
    faces = [tuple(range(n)), tuple(range(n, 2 * n))]
    faces.extend((i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n))
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    nonmanifold = sum(not edge.is_manifold for edge in bm.edges)
    degenerate = sum(face.calc_area() < 1e-8 for face in bm.faces)
    if nonmanifold or degenerate:
        raise RuntimeError(f"Invalid stair topology: {nonmanifold} nonmanifold edges; {degenerate} degenerate faces")
    bm.to_mesh(mesh)
    bm.free()
    layer = mesh.uv_layers.new(name="Projected stair atlas")
    for loop in mesh.loops:
        weights = inverse @ project(mesh.vertices[loop.vertex_index].co)
        layer.data[loop.index].uv = sum((uv[i] * weights[i] for i in range(3)), Vector((0, 0)))
    for material in source.data.materials:
        mesh.materials.append(material)
    if existing:
        obj = existing
        obj.data = mesh
        obj.matrix_world = Matrix.Identity(4)
    else:
        obj = bpy.data.objects.new(name, mesh)
        collection.objects.link(obj)
    obj["source_obstacle"] = source["source_obstacle"]
    obj["step_count"] = count
    obj["todo"] = "Refine tread spacing against the painted steps and texture concealed sides independently."
    source.hide_render = True
    source.hide_set(True)
    scene = bpy.data.scenes["Derby Refinement"]
    target = sum(points, Vector()) / 4
    for label, yaw in (("stairs-reference", 0), ("stairs-east", 40), ("stairs-west", -40)):
        camera = bpy.data.objects.get("Derby " + label)
        if camera is None:
            data = bpy.data.cameras.new("Derby " + label)
            camera = bpy.data.objects.new(data.name, data)
            bpy.data.collections["Derby Inspection"].objects.link(camera)
        data = camera.data
        yaw = math.radians(yaw)
        camera.location = target + Vector((math.sin(yaw) * cos, -math.cos(yaw) * cos, sin)) * 3000
        camera.rotation_euler = (target - camera.location).to_track_quat("-Z", "Y").to_euler()
        data.type = "ORTHO"
        data.ortho_scale = 430
        data.clip_end = 30000
    bpy.context.view_layer.update()
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    return {"object": obj.name, "steps": count, "faces": len(mesh.polygons), "nonmanifold_edges": nonmanifold, "degenerate_faces": degenerate}
