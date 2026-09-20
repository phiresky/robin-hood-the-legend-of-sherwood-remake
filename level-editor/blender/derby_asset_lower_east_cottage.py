"""Audited front hip and watertight shells for the Lower Bailey East Cottage.

The artwork shows a thatched hip at the front and exposed rear rafters. The
generated roof instead extends its ridge to a vertical front gable. Move the
front ridge endpoint back 30 percent of the roof length and lower the front
center to the eaves. Concealed rear rafters remain unresolved by this recipe.
"""
import bpy
import bmesh
import math
from mathutils import Matrix, Vector

TAG = "lower_east_cottage_refinement"
FALLBACK = "reprojection_fallback_material"


def _roof(source, number, shared_ridge):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    # The triangulated source top identifies the four physical roof corners.
    a, b, c, d = [world[i].copy() for i in ((16, 17, 18, 19) if number == 49 else (18, 17, 16, 19))]
    b, c = [v.copy() for v in shared_ridge]
    ridge = b.lerp(c, .30)
    # Use the mean eave height on both halves to meet at one front hip seam.
    b.z = (88.1369 + 86.0930) / 2
    outline = [a, b, ridge, c, d]
    vertices = outline + [Vector((v.x, v.y, 0)) for v in outline]
    faces = [(0, 1, 2), (0, 2, 3, 4), (9, 8, 7, 6, 5)]
    mappings = [8, 8, 0]
    side_sources = [2, 0, 0, 6, 4] if number == 49 else [2, 4, 4, 6, 0]
    for i in range(5):
        j = (i + 1) % 5
        faces.append((i, j, j + 5, i + 5)); mappings.append(side_sources[i])
    return _mapped_mesh(source, vertices, faces, mappings)


def _barrel(source):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    a, b, c, d = [world[i] for i in (16, 17, 18, 19)]
    center = (a + b + c + d) / 4
    u, v = (b - a) / 2, (d - a) / 2
    u.z = v.z = 0
    vertices = []
    count = 16
    for z, radius in ((0, .9), (.2, 1), (.75, 1), (1, .9)):
        for i in range(count):
            angle = 2 * math.pi * i / count
            point = center + radius * (u * math.cos(angle) + v * math.sin(angle))
            point.z = z * center.z
            vertices.append(point)
    faces = [tuple(reversed(range(count))), tuple(range(3 * count, 4 * count))]
    mappings = [8, 8]
    for ring in range(3):
        for i in range(count):
            j = (i + 1) % count
            faces.append((ring * count + i, ring * count + j,
                          (ring + 1) * count + j, (ring + 1) * count + i))
            # The visible front half uses the front source wall; the opposite
            # half keeps its existing concealed-side fallback.
            angle = 2 * math.pi * (i + .5) / count
            mappings.append(0 if math.cos(angle) > .707 else 6 if math.sin(angle) > .707
                            else 4 if math.cos(angle) < -.707 else 2)
    return _mapped_mesh(source, vertices, faces, mappings)


def _mapped_mesh(source, vertices, faces, mappings):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    mesh = bpy.data.meshes.new(source.data.name + " / reviewed geometry")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    for material in source.data.materials:
        mesh.materials.append(material)
    def project(v):
        return Vector((v.x, -v.y * math.sin(math.radians(35)) - v.z * math.cos(math.radians(35)), 1))
    # Retain all atlas/projection channels for the next visibility-aware pass.
    for old_uv in source.data.uv_layers:
        uv = mesh.uv_layers.new(name=old_uv.name)
        for face, index in zip(mesh.polygons, mappings):
            old = source.data.polygons[index]
            inverse = Matrix([project(world[i]) for i in old.vertices]).transposed().inverted()
            old_values = [old_uv.data[i].uv.copy() for i in old.loop_indices]
            for loop in face.loop_indices:
                weights = inverse @ project(vertices[mesh.loops[loop].vertex_index])
                uv.data[loop].uv = sum((old_values[i] * weights[i] for i in range(3)), Vector((0, 0)))
    backup = mesh.attributes.new(FALLBACK, "INT", "FACE")
    old_backup = source.data.attributes.get(FALLBACK)
    for face, index in zip(mesh.polygons, mappings):
        material = old_backup.data[index].value if old_backup else source.data.polygons[index].material_index
        backup.data[face.index].value = material
        face.material_index = material
    return mesh


def refine():
    collection = bpy.data.collections["Derby Working"]
    bpy.context.view_layer.update()
    previous = [o for o in collection.all_objects if o.get(TAG)]
    if previous:
        if {o.get("source_node") for o in previous} != {f"building-{i:03}" for i in range(49, 53)}:
            raise ValueError("Incomplete lower east cottage refinement")
        return {"status": "existing", "objects": [o.name for o in previous]}
    originals = {}
    for number in range(49, 53):
        found = [o for o in collection.all_objects if o.type == "MESH" and not o.hide_render
                 and o.get("source_node") == f"building-{number:03}"]
        if len(found) != 1:
            raise ValueError(f"Expected one cottage source {number}: {len(found)}")
        originals[number] = found[0]
    left, right = originals[49], originals[50]
    shared_ridge = ((left.matrix_world @ left.data.vertices[17].co + right.matrix_world @ right.data.vertices[17].co) / 2,
                    (left.matrix_world @ left.data.vertices[18].co + right.matrix_world @ right.data.vertices[16].co) / 2)
    report = []
    for number, source in originals.items():
        mesh = (_roof(source, number, shared_ridge) if number in (49, 50)
                else _barrel(source) if number == 51 else source.data.copy())
        bm = bmesh.new()
        try:
            bm.from_mesh(mesh)
            if number == 52:
                bm.transform(source.matrix_world)
                bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.6)
                edges = [e for e in bm.edges if e.is_boundary]
                if any(abs(v.co.z) > .01 for e in edges for v in e.verts):
                    raise ValueError(f"Unexpected cottage seam: {source.name}")
                bmesh.ops.holes_fill(bm, edges=edges, sides=0)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            bmesh.ops.triangulate(bm, faces=list(bm.faces))
            defects = {"nonmanifold": sum(not e.is_manifold for e in bm.edges),
                       "degenerate": sum(f.calc_area() < 1e-6 for f in bm.faces)}
            if any(defects.values()):
                raise ValueError(f"Invalid cottage geometry: {defects}")
            bm.to_mesh(mesh)
        finally:
            bm.free()
        replacement = bpy.data.objects.new(source.name + " / reviewed shell", mesh)
        collection.objects.link(replacement)
        replacement.parent = source.parent
        for key in source.keys():
            replacement[key] = source[key]
        replacement[TAG] = "front-hip-and-closed-shells-v1"
        replacement.matrix_world = Matrix.Identity(4)
        source.hide_render = source.hide_viewport = True
        report.append({"source_node": source["source_node"], "faces": len(mesh.polygons),
                       "validation": defects})
    bpy.context.view_layer.update()
    return {"status": "created", "objects": report}
