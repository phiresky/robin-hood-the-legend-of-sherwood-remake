"""Source-audited East Bailey gate arch, crenels and complete turret roof.

Run refine() before layered reprojection. Render meshes are independent from
collision records; original components remain hidden and canonical IDs survive.
"""
import math
from pathlib import Path
import bpy
import bmesh
from mathutils import Matrix, Vector
from derby_asset_lower_east_curtain import _rebuild, _stone_donor
from derby_asset_lower_east_cottage import _mapped_mesh

ASSET = "derby-east-bailey-gate"
TAG = "east_bailey_gate_refinement"
IDS = (76, 79, 80, 83, 84, 85, 95, 96)


def _validate(obj):
    bm = bmesh.new()
    try:
        bm.from_mesh(obj.data)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        defects = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                   "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
        if any(defects.values()):
            raise ValueError(f"Invalid gate shell {obj.name}: {defects}")
        bm.to_mesh(obj.data)
        return defects
    finally:
        bm.free()


def _replacement(source, mesh):
    obj = bpy.data.objects.new(source.name + " / reviewed geometry", mesh)
    bpy.data.collections["Derby Working"].objects.link(obj)
    obj.parent = source.parent
    for key in source.keys():
        obj[key] = source[key]
    obj.matrix_world = Matrix.Identity(4)
    source.hide_render = source.hide_viewport = True
    return obj


def _stone_fallback(obj):
    """Concealed masonry must not stretch the painted upper doorway downward."""
    index = len(obj.data.materials)
    obj.data.materials.append(_stone_donor())
    uv = obj.data.uv_layers.get("UVMap") or obj.data.uv_layers.new(name="UVMap")
    backup = obj.data.attributes.get("reprojection_fallback_material") or obj.data.attributes.new("reprojection_fallback_material", "INT", "FACE")
    for face in obj.data.polygons:
        face.material_index = index
        backup.data[face.index].value = index
        axis = max(range(3), key=lambda i: abs(face.normal[i]))
        axes = [i for i in range(3) if i != axis]
        for loop in face.loop_indices:
            point = obj.data.vertices[obj.data.loops[loop].vertex_index].co
            uv.data[loop].uv = (point[axes[0]] / 80, point[axes[1]] / 45)


def _roof_fallback(obj):
    name = "East Bailey gate / concealed roof tile donor"
    material = bpy.data.materials.get(name)
    if material is None:
        path = Path(__file__).resolve().parent.parent / "work/derby-refinement/interior-layers/covered.png"
        source = bpy.data.images.load(str(path), check_existing=True)
        width, height = source.size
        x, y, size = 1150, height - 1500 - 16, 16
        pixels = []
        for row in range(y, y + size):
            start = (row * width + x) * 4
            pixels.extend(source.pixels[start:start + size * 4])
        image = bpy.data.images.new(name, width=size, height=size, alpha=True)
        image.pixels = pixels; image.pack()
        material = bpy.data.materials.new(name); material.use_nodes = True
        nodes = material.node_tree.nodes; nodes.clear()
        uvnode = nodes.new("ShaderNodeUVMap"); uvnode.uv_map = "UVMap"
        texture = nodes.new("ShaderNodeTexImage"); texture.image = image
        emission = nodes.new("ShaderNodeEmission"); output = nodes.new("ShaderNodeOutputMaterial")
        for output_socket, input_socket in ((uvnode.outputs[0], texture.inputs[0]),
                                            (texture.outputs[0], emission.inputs[0]),
                                            (emission.outputs[0], output.inputs[0])):
            material.node_tree.links.new(output_socket, input_socket)
    index = len(obj.data.materials); obj.data.materials.append(material)
    uv = obj.data.uv_layers["UVMap"]
    backup = obj.data.attributes.get("reprojection_fallback_material") or obj.data.attributes.new("reprojection_fallback_material", "INT", "FACE")
    for face in obj.data.polygons:
        face.material_index = index; backup.data[face.index].value = index
        axis = max(range(3), key=lambda i: abs(face.normal[i]))
        axes = [i for i in range(3) if i != axis]
        for loop in face.loop_indices:
            p = obj.data.vertices[obj.data.loops[loop].vertex_index].co
            uv.data[loop].uv = (p[axes[0]] / 16, p[axes[1]] / 16)


def _corner_turret(body, roof, center):
    """Round corner tower and its small pointed roof visible beside the doorway."""
    count = 24
    vertices = []
    for z in (0, 259):
        vertices += [Vector((center.x + 18 * math.cos(2 * math.pi * i / count),
                             center.y + 18 * math.sin(2 * math.pi * i / count), z)) for i in range(count)]
    faces = [tuple(reversed(range(count))), tuple(range(count, count * 2))]
    faces += [(i, (i + 1) % count, (i + 1) % count + count, i + count) for i in range(count)]
    mesh = _mapped_mesh(body, vertices, faces, [8] * len(faces))
    turret = _replacement(body, mesh)
    turret.name = "East Bailey Gatehouse / Round corner turret"
    _validate(turret)
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=5, use_grid_fill=True)
    bm.to_mesh(mesh); bm.free()
    _stone_fallback(turret)
    ring = [Vector((center.x + 21 * math.cos(2 * math.pi * i / count),
                    center.y + 21 * math.sin(2 * math.pi * i / count), 259)) for i in range(count)]
    apex = Vector((center.x, center.y, 335))
    faces = [tuple(reversed(range(count)))] + [(i, (i + 1) % count, count) for i in range(count)]
    cone = _replacement(roof, _mapped_mesh(roof, ring + [apex], faces, [6] * len(faces)))
    cone.name = "East Bailey Gatehouse / Round corner turret conical roof"
    return turret, cone


def _arch_points(source):
    points = [source.matrix_world @ v.co for v in source.data.vertices]
    left, right = points[18], points[19]
    back = points[20] - right
    back.z = 0
    base = min(p.z for p in points)
    radius = (right - left).length / 2
    profile = []
    for i in range(17):
        u = i / 16
        point = left.lerp(right, u)
        point.z = base + radius * math.sin(math.pi * u)
        profile.append(point)
    return points, profile, back


def _arch(source):
    points, profile, back = _arch_points(source)
    front = profile + [points[19], points[18]]
    count = len(front)
    vertices = front + [p + back for p in front]
    faces = [tuple(range(count)), tuple(reversed(range(count, count * 2)))]
    mappings = [2, 6]
    for i in range(count):
        j = (i + 1) % count
        faces.append((i, j, j + count, i + count))
        mappings.append(8 if i == count - 2 else 2)
    return _replacement(source, _mapped_mesh(source, vertices, faces, mappings))


def _cut_matching_arch(obj, source):
    """Remove the parapet's overlapping flat lintel behind the same arch."""
    points, profile, back = _arch_points(source)
    normal = back.normalized()
    front = [p - normal * 20 for p in profile]
    front += [Vector((front[-1].x, front[-1].y, 0)),
              Vector((front[0].x, front[0].y, 0))]
    count = len(front)
    vertices = front + [p + normal * (back.length + 40) for p in front]
    faces = [tuple(range(count)), tuple(reversed(range(count, count * 2)))]
    faces += [(i, (i + 1) % count, (i + 1) % count + count, i + count) for i in range(count)]
    mesh = bpy.data.meshes.new("Gate arch temporary cutter")
    mesh.from_pydata(vertices, [], faces)
    cutter = bpy.data.objects.new(mesh.name, mesh)
    bpy.context.scene.collection.objects.link(cutter)
    _validate(cutter)
    try:
        mod = obj.modifiers.new("Continuation of gate arch", "BOOLEAN")
        mod.operation, mod.solver, mod.object = "DIFFERENCE", "EXACT", cutter
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=mod.name)
    finally:
        bpy.data.objects.remove(cutter, do_unlink=True)
        bpy.data.meshes.remove(mesh)


def refine():
    working = bpy.data.collections["Derby Working"]
    bpy.context.view_layer.update()
    existing = [o for o in working.all_objects if o.get(TAG)]
    if existing:
        if len(existing) != len(IDS) + 2 or {o.get("source_node") for o in existing} != {f"building-{n:03}" for n in IDS}:
            raise ValueError("Incomplete East Bailey gate refinement")
        return {"status": "existing", "objects": len(existing)}
    sources = {}
    for number in IDS:
        found = [o for o in working.all_objects if o.type == "MESH" and not o.hide_render
                 and o.get("source_node") == f"building-{number:03}"]
        if len(found) != 1:
            raise ValueError(f"Expected one gate source {number}, found {len(found)}")
        sources[number] = found[0]
    # Resolve all source transforms before hiding originals disables updates.
    world = {n: [o.matrix_world @ v.co for v in o.data.vertices] for n, o in sources.items()}
    report = []
    for number in IDS:
        source = sources[number]
        if number in (79, 80):
            body = world[76]
            apex = (world[79][13] + world[80][13]) / 2
            corners = (18, 19, 17) if number == 79 else (19, 16, 17)
            vertices = [body[i] for i in corners] + [apex]
            # Two closed tetrahedra form one complete pyramid at the turret
            # eaves, replacing opaque roof columns that reached the ground.
            faces = [(0, 1, 2), (0, 3, 1), (1, 3, 2), (2, 3, 0)]
            mesh = _mapped_mesh(source, vertices, faces, [6, 6, 6, 6])
            # Concealed roof planes use the full original roof donor rather
            # than extrapolating far beyond its atlas island.
            old = source.data.polygons[6]
            for uv in mesh.uv_layers:
                original = source.data.uv_layers[uv.name]
                coords = [original.data[i].uv.copy() for i in old.loop_indices]
                for face in mesh.polygons:
                    for index, loop in enumerate(face.loop_indices):
                        uv.data[loop].uv = coords[index]
            obj = _replacement(source, mesh)
        elif number == 83:
            obj = _arch(source)
        else:
            spans = []
            if number == 95:
                spans = [(35, 36, [(.17, .38), (.58, .80)], 26),
                         (34, 35, [(.30, .65)], 26)]
            elif number == 96:
                spans = [(22, 23, [(.02, .11), (.22, .37), (.50, .67), (.80, .93)], 26)]
            _rebuild(source, spans)
            obj = next(o for o in working.all_objects if o.get("source_node") == source["source_node"]
                       and o != source and not o.hide_render)
            if number == 96:
                _cut_matching_arch(obj, sources[83])
        obj[TAG] = "arch-crenels-complete-roof-v1"
        source.hide_render = source.hide_viewport = True
        validation = _validate(obj)
        if number not in (79, 80):
            _stone_fallback(obj)
        else:
            bm = bmesh.new(); bm.from_mesh(obj.data)
            bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=6, use_grid_fill=True)
            bm.to_mesh(obj.data); bm.free()
            _roof_fallback(obj)
        fallback = obj.data.attributes.get("reprojection_fallback_material")
        if fallback is None:
            fallback = obj.data.attributes.new("reprojection_fallback_material", "INT", "FACE")
            for face in obj.data.polygons:
                fallback.data[face.index].value = face.material_index
        report.append({"source_node": source["source_node"], "faces": len(obj.data.polygons), **validation})
    for obj in _corner_turret(sources[76], sources[80], world[76][19]):
        obj[TAG] = "arch-crenels-complete-roof-v1"
        if obj["source_node"] == "building-080":
            _roof_fallback(obj)
        report.append({"source_node": obj["source_node"], "name": obj.name, **_validate(obj)})
    bpy.context.view_layer.update()
    return {"asset": ASSET, "status": "created", "crenels": 7, "objects": report}
