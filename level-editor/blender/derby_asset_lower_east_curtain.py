"""Measured crenels for the lower east curtain, retaining original source shells.

The wall walk and twenty-step flight are intentionally preserved. Opening
positions use covered-artwork coordinates; newly exposed stone uses atlas donors
until the normal visibility-aware reprojection pass runs.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

ASSET = "derby-lower-east-curtain"


def _stone_donor():
    """Pack a clean masonry crop for the source camera's nearly edge-on walls."""
    name = "Lower east curtain / concealed masonry donor"
    material = bpy.data.materials.get(name)
    if material:
        return material
    atlas = bpy.data.images["atlas"]
    if tuple(atlas.size) != (8192, 8192):
        raise ValueError("The reviewed curtain masonry donor requires the Derby atlas")
    x, y, width, height = 6200, 8192 - 1725 - 45, 80, 45
    pixels = []
    for row in range(y, y + height):
        start = (row * atlas.size[0] + x) * 4
        pixels.extend(atlas.pixels[start:start + width * 4])
    image = bpy.data.images.new(name, width=width, height=height, alpha=True)
    image.pixels = pixels
    image.pack()
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    nodes.clear()
    output = nodes.new("ShaderNodeOutputMaterial")
    emission = nodes.new("ShaderNodeEmission")
    texture = nodes.new("ShaderNodeTexImage")
    texture.image = image
    uv = nodes.new("ShaderNodeUVMap")
    uv.uv_map = "UVMap"
    material.node_tree.links.new(uv.outputs[0], texture.inputs[0])
    material.node_tree.links.new(texture.outputs[0], emission.inputs[0])
    material.node_tree.links.new(emission.outputs[0], output.inputs[0])
    return material


def _rebuild(source, spans):
    world = source.matrix_world.copy()
    points = [world @ vertex.co for vertex in source.data.vertices]
    top = max(p.z for p in points)
    # The export has independent wall-face vertices. Its top triangulation is
    # the authoritative continuous footprint, including the tower returns.
    top_faces = [p for p in source.data.polygons
                 if all(abs(points[v].z - top) < .25 for v in p.vertices)]
    edges = {}
    for face in top_faces:
        for a, b in zip(face.vertices, list(face.vertices[1:]) + [face.vertices[0]]):
            key = tuple(sorted((a, b)))
            edges[key] = edges.get(key, 0) + 1
    boundary = [edge for edge, count in edges.items() if count == 1]
    adjacency = {}
    for a, b in boundary:
        adjacency.setdefault(a, []).append(b)
        adjacency.setdefault(b, []).append(a)
    if not adjacency or any(len(v) != 2 for v in adjacency.values()):
        raise ValueError(f"Ambiguous top perimeter: {source.name}")
    order = [min(adjacency)]
    previous = None
    while True:
        following = next(v for v in adjacency[order[-1]] if v != previous)
        if following == order[0]:
            break
        previous = order[-1]
        order.append(following)
        if len(order) > len(adjacency):
            raise ValueError("Nonclosing wall perimeter")
    if len(order) != len(adjacency):
        raise ValueError("Multiple wall perimeter loops")
    bottom = min(p.z for p in points)
    vertices = [(points[i].x, points[i].y, z) for z in (bottom, top) for i in order]
    n = len(order)
    faces = [tuple(range(n)), tuple(range(n, n * 2))]
    faces += [(i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n)]
    mesh = bpy.data.meshes.new(source.name + " / closed parapet")
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(source.name + " / open crenels", mesh)
    bpy.data.collections["Derby Working"].objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith("reprojection_"):
            obj[key] = source[key]
    bpy.context.view_layer.update()
    for start, end, intervals, depth in spans:
        a, b = points[start], points[end]
        along = (b - a).normalized()
        for low, high in intervals:
            center = a.lerp(b, (low + high) / 2)
            center.z = top + (10 - depth) / 2
            bpy.ops.mesh.primitive_cube_add(size=1, location=center)
            cutter = bpy.context.object
            cutter.name = "Temporary lower east crenel cutter"
            cutter.rotation_euler.z = math.atan2(along.y, along.x)
            cutter.dimensions = ((b - a).length * (high - low), 35, depth + 10)
            bpy.context.view_layer.update()
            mod = obj.modifiers.new("Measured parapet opening", "BOOLEAN")
            mod.operation, mod.solver, mod.object = "DIFFERENCE", "EXACT", cutter
            bpy.context.view_layer.objects.active = obj
            obj.select_set(True)
            bpy.ops.object.modifier_apply(modifier=mod.name)
            bpy.data.objects.remove(cutter, do_unlink=True)
    # Transfer the original atlas, never the last projection material. Newly
    # cut horizontal/reveal faces use a small stone sample below the opening.
    donors = []
    normal_matrix = world.to_3x3().inverted().transposed()
    fallback = source.data.attributes.get("reprojection_fallback_material")
    for face in source.data.polygons:
        if len(face.vertices) != 3:
            continue
        triangle = [points[i] for i in face.vertices]
        edge_a, edge_b = triangle[1] - triangle[0], triangle[2] - triangle[0]
        normal = edge_a.cross(edge_b).normalized()
        matrix = Matrix((edge_a, edge_b, normal)).transposed()
        if abs(matrix.determinant()) < 1e-6:
            continue
        donors.append(((normal_matrix @ face.normal).normalized(),
                       world @ face.center, matrix.inverted(), triangle[0],
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices],
                       fallback.data[face.index].value if fallback else face.material_index))
    obj.data.materials.clear()
    for material in source.data.materials:
        obj.data.materials.append(material)
    uv = obj.data.uv_layers.get("UVMap") or obj.data.uv_layers.new(name="UVMap")
    obj.data.uv_layers.active = uv
    uv.active_render = True
    donor_index = len(obj.data.materials)
    obj.data.materials.append(_stone_donor())
    for face in obj.data.polygons:
        if abs(face.normal.x) > .98 and abs(face.normal.z) < .1:
            face.material_index = donor_index
            for loop in face.loop_indices:
                point = obj.data.vertices[obj.data.loops[loop].vertex_index].co
                uv.data[loop].uv = (point.y / 80, point.z / 45)
            continue
        donor = max(donors, key=lambda d: d[0].dot(face.normal) * 10000 - (d[1] - face.center).length)
        normal, center, inverse, origin, coords, material = donor
        face.material_index = material
        reveal = min(obj.data.vertices[i].co.z for i in face.vertices) > top - 50
        for loop in face.loop_indices:
            point = obj.data.vertices[obj.data.loops[loop].vertex_index].co
            if reveal and normal.dot(face.normal) < .98:
                point = face.center + (point - face.center) * .1
                point.z = top - 65
            bary = inverse @ (point - origin)
            weights = Vector((1 - bary.x - bary.y, bary.x, bary.y))
            if reveal and normal.dot(face.normal) < .98:
                weights = Vector([max(.02, min(.96, w)) for w in weights])
                weights /= sum(weights)
            uv.data[loop].uv = sum((coords[i] * weights[i] for i in range(3)), Vector((0, 0)))
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    # Local cells let the projection pass keep hidden regions on the donor
    # atlas without rejecting an entire long wall behind one merlon.
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bmesh.ops.subdivide_edges(bm, edges=list(bm.edges), cuts=3,
                             use_grid_fill=True, smooth=0)
    bm.normal_update()
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-7 for f in bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    if bad_edges or bad_faces:
        raise ValueError(f"Invalid curtain topology: {bad_edges} edges, {bad_faces} faces")
    obj["lower_east_crenels"] = sum(len(span[2]) for span in spans)
    obj["lower_east_rebuilt"] = True
    obj["projection_min_cosine"] = .15
    obj["concealed_surface_note"] = "New crenel reveals retain a local stone atlas donor."
    source["lower_east_refined"] = True
    source.hide_render = True
    source.hide_set(True)
    return {"source": source["source_node"], "openings": obj["lower_east_crenels"],
            "nonmanifold_edges": bad_edges, "degenerate_faces": bad_faces}


def refine():
    bpy.context.view_layer.update()
    working = bpy.data.collections["Derby Working"]
    specs = {
        # Top edges follow the upper wall from its lower return to the gate.
        "building-010": [(22, 21, [(0.12,.22),(.34,.44),(.56,.66),(.78,.88)], 27)],
        # Long near-vertical run: fourteen distinct painted stone merlons.
        "building-007": [(41,40, [(i/14+.037,i/14+.067) for i in range(13)], 27),
                         (45,46, [(.33,.64)], 27)],
        "building-041": [(61,60, [(.15,.30),(.45,.60),(.75,.90)], 27),
                         (64,71, [(.3,.65)], 27),
                         (71,72, [(.3,.65)], 27),
                         (72,65, [(.3,.65)], 27),
                         (65,66, [(.3,.65)], 27)],
        # Continuous low railing and walking platform: close their imported
        # triangle seams without inventing openings in continuous painted stone.
        "building-024": [],
        "building-044": [],
    }
    report = []
    for node, spans in specs.items():
        source = next(o for o in working.objects if o.get("source_node") == node
                      and not o.get("lower_east_rebuilt"))
        if source.get("lower_east_refined"):
            report.append({"source": node, "skipped": "already refined"})
        else:
            report.append(_rebuild(source, spans))
    return {"asset": ASSET, "changes": report,
            "preserved": ["building-012"]}
