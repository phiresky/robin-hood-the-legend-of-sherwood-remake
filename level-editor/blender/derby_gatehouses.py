"""Restore the south gate's painted arch as a closed, separately owned mesh."""
import math
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector


def refine(source_image_path, segments=24):
    """Replace the flat passage lintel; repeated calls reuse the existing result.

    The image shows a round arch whose crown coincides with the old lintel.
    Other gatehouse chambers and their reveal geometry remain independent.
    This does not save a checkpoint: the caller owns integration and saving.
    """
    working = bpy.data.collections["Derby Working"]
    tag = "south-gate-round-arch-v1"
    existing = [o for o in working.objects if o.get("gate_refinement") == tag]
    if existing:
        if len(existing) != 1:
            raise RuntimeError("Multiple south gate arch replacements")
        return {"object": existing[0].name, "reused": True}
    if segments < 8:
        raise ValueError("At least eight arch segments are required")
    image_path = Path(source_image_path).resolve()
    if not image_path.is_file():
        raise FileNotFoundError(image_path)
    sources = [o for o in working.objects if o.type == "MESH"
               and o.get("source_node") == "building-002" and not o.hide_render]
    if len(sources) != 1:
        raise RuntimeError("Expected one unrefined south gate lintel")
    source = sources[0]
    if len(source.data.vertices) != 20 or len(source.data.polygons) != 10:
        raise ValueError("South gate lintel topology changed; re-audit arch anchors")
    bpy.context.view_layer.update()
    points = [source.matrix_world @ v.co for v in source.data.vertices]
    # The front/back boundaries follow the existing piers and roof exactly.
    front_left, front_right = points[7], points[5]
    back_left, back_right = points[13], points[12]
    crown = (front_left.z + front_right.z) / 2
    radius = (front_right - front_left).length / 2
    spring = crown - radius
    top_front, top_back = points[4].z, points[14].z
    if not 35 < radius < 50 or not 105 < crown < 120:
        raise ValueError("Gate dimensions changed; re-audit source arch")
    profile = [(0.0, top_front), (1.0, top_front)]
    # Right spring, crown, left spring. The underside follows a semicircle.
    for i in range(segments + 1):
        angle = math.pi * i / segments
        profile.append(((1 + math.cos(angle)) / 2,
                        spring + radius * math.sin(angle)))
    vertices = []
    for left, right, top in ((front_left, front_right, top_front),
                             (back_left, back_right, top_back)):
        for index, (t, z) in enumerate(profile):
            p = left.lerp(right, t)
            p.z = top if index < 2 else z
            vertices.append(p)
    n = len(profile)
    faces = [tuple(range(n)), tuple(range(n, 2 * n))]
    faces.extend((i, (i + 1) % n, (i + 1) % n + n, i + n)
                 for i in range(n))
    mesh = bpy.data.meshes.new("South gate arched passage")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not edge.is_manifold for edge in bm.edges)
    bad_faces = sum(face.calc_area() < 1e-8 for face in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad_edges or bad_faces:
        bpy.data.meshes.remove(mesh)
        raise RuntimeError("Generated arch is not closed and nondegenerate")
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def project(p):
        return Vector((p.x, -p.y * sine - p.z * cosine, 1))

    # Preserve atlas material coordinates for the concealed surfaces. New
    # haunches extend below the old crop, so clamp fallback sampling to masonry.
    mappings = {}
    for face_id in (0, 2, 4, 6, 8):
        face = source.data.polygons[face_id]
        inverse = Matrix([project(points[i]) for i in face.vertices]).transposed().inverted()
        uvs = [source.data.uv_layers.active.data[i].uv.copy() for i in face.loop_indices]
        mappings[face_id] = (inverse, uvs)
    fallback = mesh.uv_layers.new(name=source.data.uv_layers.active.name)
    reference = mesh.uv_layers.new(name="Gate reference projection")
    for polygon in mesh.polygons:
        face_id = {0: 2, 1: 6, 2: 8, 3: 0, n + 1: 4}.get(polygon.index, 2)
        inverse, uvs = mappings[face_id]
        for loop_id in polygon.loop_indices:
            p = mesh.vertices[mesh.loops[loop_id].vertex_index].co
            q = p.copy()
            q.z = max(crown + 2, q.z)
            weights = inverse @ project(q)
            fallback.data[loop_id].uv = sum((uvs[i] * weights[i] for i in range(3)), Vector((0, 0)))
            pixel = project(p)
            reference.data[loop_id].uv = (pixel.x / 1920, 1 - pixel.y / 2752)
    mesh.uv_layers.active_index = 0
    for material in source.data.materials:
        mesh.materials.append(material)
    material = bpy.data.materials.new("South gate source-facing arch projection")
    material.use_nodes = True
    nodes = material.node_tree.nodes
    nodes.clear()
    output = nodes.new("ShaderNodeOutputMaterial")
    emission = nodes.new("ShaderNodeEmission")
    texture = nodes.new("ShaderNodeTexImage")
    texture.image = bpy.data.images.load(str(image_path), check_existing=True)
    if tuple(texture.image.size) != (1920, 2752):
        raise ValueError("Expected a 1920 by 2752 Derby projection source")
    uv = nodes.new("ShaderNodeUVMap")
    uv.uv_map = reference.name
    links = material.node_tree.links
    links.new(uv.outputs["UV"], texture.inputs["Vector"])
    links.new(texture.outputs["Color"], emission.inputs["Color"])
    links.new(emission.outputs[0], output.inputs["Surface"])
    mesh.materials.append(material)
    # Only the south facade uses source art. Concealed/reveal-facing geometry
    # keeps its atlas fallback until the visibility-aware reprojection pass.
    mesh.polygons[0].material_index = len(mesh.materials) - 1
    obj = bpy.data.objects.new(source.name + " / arched passage", mesh)
    working.objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        obj[key] = source[key]
    obj["gate_refinement"] = tag
    obj["arch_segments"] = segments
    obj["todo"] = "Reproject against the final closed exterior and its visibility masks."
    source.hide_render = True
    source.hide_set(True)
    source["gate_refinement_baseline"] = tag
    bpy.context.view_layer.update()
    return {"object": obj.name, "source_node": obj["source_node"],
            "arch_segments": segments, "spring_height": spring,
            "crown_height": crown, "radius": radius,
            "nonmanifold_edges": bad_edges, "degenerate_faces": bad_faces}
