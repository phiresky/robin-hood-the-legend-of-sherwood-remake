"""Replace the postern's solid ladder proxies with source-aligned timber ladders.

The covered artwork at pixels (450,2190)-(750,2490) shows two open ladders,
with stone and air visible between their rungs. They are not masonry stairs.
The collision source records remain unchanged; these are render replacements.
"""
import math

import bpy
import bmesh
from mathutils import Matrix, Vector

ASSET = "derby-southwest-postern"
TAG = "postern-open-timber-ladders-v1"


def _parapet(working):
    tag = "postern-rear-parapet-v1"
    existing = next((o for o in working.objects if o.get("postern_parapet") == tag), None)
    if existing:
        return {"reused": True, "object": existing.name}
    source = next(o for o in working.objects if o.get("source_node") == "building-008" and not o.hide_render)
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    front = [world[18], world[19]]
    back = [world[17], world[16]]
    top = sum(p.z for p in front + back) / 4
    # Two clearly exposed gaps in the source crop (625,2225)-(745,2300).
    # The rightmost wall disappears into the gate tower and stays unchanged.
    x0 = (front[0].x + back[0].x) / 2
    x1 = (front[1].x + back[1].x) / 2
    profile = [(0, 0), (1, 0), (1, top)]
    for left, right in ((686, 697), (653, 667)):
        a, b = (left-x0)/(x1-x0), (right-x0)/(x1-x0)
        profile.extend(((b,top),(b,top-23),(a,top-23),(a,top)))
    profile.append((0,top))
    vertices=[]
    for left,right in (front,back):
        for t,z in profile:
            p=left.lerp(right,t); p.z=z; vertices.append(p)
    n=len(profile)
    faces=[tuple(range(n)),tuple(range(n,2*n))]
    faces.extend((i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n))
    mesh=bpy.data.meshes.new("Southwest postern rear parapet with open crenels")
    mesh.from_pydata(vertices,[],faces);mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free()
    def screen(p):
        return Vector((p.x,-p.y*math.sin(math.radians(35))-p.z*math.cos(math.radians(35)),1))
    for old_uv in source.data.uv_layers:
        new_uv=mesh.uv_layers.new(name=old_uv.name)
        for face in mesh.polygons:
            face_id=2 if face.index==0 else 6 if face.index==1 else 8
            original=source.data.polygons[face_id]
            inverse=Matrix([screen(world[i]) for i in original.vertices]).transposed().inverted()
            tex=[old_uv.data[i].uv.copy() for i in original.loop_indices]
            for loop_id in face.loop_indices:
                weights=inverse @ screen(vertices[mesh.loops[loop_id].vertex_index])
                new_uv.data[loop_id].uv=sum((tex[i]*weights[i] for i in range(3)),Vector((0,0)))
    for mat in source.data.materials:mesh.materials.append(mat)
    fallback=source.data.attributes.get("reprojection_fallback_material")
    for face in mesh.polygons:
        i=2 if face.index==0 else 6 if face.index==1 else 8
        face.material_index=fallback.data[i].value if fallback else source.data.polygons[i].material_index
    obj=bpy.data.objects.new("Southwest Postern Tower / Rear parapet with crenels",mesh)
    working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith("reprojection_"):obj[key]=source[key]
    obj["postern_parapet"]=tag
    source.hide_render=source.hide_viewport=True
    source["postern_parapet_baseline"]=tag
    bm=bmesh.new();bm.from_mesh(mesh)
    bad={"nonmanifold_edges":sum(not e.is_manifold for e in bm.edges),
         "degenerate_faces":sum(f.calc_area()<1e-7 for f in bm.faces)}
    bm.free()
    if any(bad.values()):raise ValueError(bad)
    return {"source_node":"building-008","crenels":2,**bad}


def refine():
    working = bpy.data.collections["Derby Working"]
    bpy.context.view_layer.update()
    parapet = _parapet(working)
    existing = [o for o in working.objects if o.get("postern_refinement") == TAG]
    if existing:
        if sorted(o.get("source_node") for o in existing) != ["building-039", "building-040"]:
            raise ValueError("Incomplete postern ladder replacement")
        return {"reused": True, "objects": [o.name for o in existing], "parapet": parapet}
    bpy.context.view_layer.update()
    results = []
    for node, top_face, count in (("building-039", 6, 14), ("building-040", 8, 8)):
        sources = [o for o in working.objects if o.type == "MESH"
                   and o.get("asset_group") == ASSET and o.get("source_node") == node
                   and not o.hide_render]
        source = next(o for o in sources if not o.get("step_count"))
        # Each audited ramp has a four-corner top, split into two triangles.
        ids = set(source.data.polygons[top_face].vertices)
        ids.update(source.data.polygons[top_face + 1].vertices)
        if len(ids) != 4:
            raise ValueError("Ladder proxy top changed; re-audit anchors")
        points = sorted((source.matrix_world @ source.data.vertices[i].co for i in ids),
                        key=lambda p: p.z)
        low = sorted(points[:2], key=lambda p: p.x)
        high = sorted(points[2:], key=lambda p: p.x)
        axis = ((high[0] + high[1]) - (low[0] + low[1])).normalized()
        across = (low[1] - low[0]).normalized()
        normal = across.cross(axis).normalized()
        vertices, faces = [], []

        def beam(a, b, width, depth):
            direction = (b - a).normalized()
            side = normal.cross(direction).normalized() * width / 2
            thick = direction.cross(side).normalized() * depth / 2
            start = len(vertices)
            vertices.extend([p + s * side + t * thick for p in (a, b)
                             for s, t in ((-1, -1), (1, -1), (1, 1), (-1, 1))])
            faces.extend(tuple(start + i for i in f) for f in
                         ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4),
                          (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)))

        # Rails follow the ramp's side boundaries, inset by their own half-width.
        rail = 1.65 if node == "building-039" else 1.3
        low = [low[0] + across * rail / 2, low[1] - across * rail / 2]
        high = [high[0] + across * rail / 2, high[1] - across * rail / 2]
        for i in range(2):
            beam(low[i], high[i], rail, rail)
        for i in range(1, count + 1):
            t = i / (count + 1)
            beam(low[0].lerp(high[0], t), low[1].lerp(high[1], t), 1.15, 1.35)
        if node == "building-039":
            # The lower ladder is lashed to a braced scaffold beneath the landing.
            # Its rear uprights meet the same landing corners as the ladder rails.
            feet = [Vector((p.x, p.y, 0.0)) for p in high]
            for i in range(2):
                beam(feet[i], high[i], 1.5, 1.5)
                beam(low[i], high[i] - Vector((0, 0, 32)), 1.1, 1.1)
                beam(feet[i] + Vector((0, 0, 32)), low[i].lerp(high[i], .65), 1.1, 1.1)
        mesh = bpy.data.meshes.new(node + " open timber ladder")
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        uv = mesh.uv_layers.new(name="UVMap")
        for loop in mesh.loops:
            p = mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv = (p.x / 1920,
                1 + (p.y * math.sin(math.radians(35)) + p.z * math.cos(math.radians(35))) / 2752)
        material = bpy.data.materials.get("Postern concealed aged timber")
        if material is None:
            material = bpy.data.materials.new("Postern concealed aged timber")
            material.diffuse_color = (0.19, 0.14, 0.075, 1)
            material.use_nodes = True
            nodes = material.node_tree.nodes
            shader = nodes.new("ShaderNodeEmission")
            shader.inputs["Color"].default_value = material.diffuse_color
            material.node_tree.links.new(shader.outputs[0], nodes.get("Material Output").inputs["Surface"])
        mesh.materials.append(material)
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bad = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
               "degenerate_faces": sum(f.calc_area() < 1e-7 for f in bm.faces)}
        bm.to_mesh(mesh)
        bm.free()
        if any(bad.values()):
            raise ValueError(bad)
        obj = bpy.data.objects.new("Southwest Postern Tower / " +
                                  ("Lower" if node == "building-039" else "Upper") +
                                  " timber ladder", mesh)
        working.objects.link(obj)
        obj.parent = source.parent
        obj.matrix_world = Matrix.Identity(4)
        for key in source.keys():
            if not key.startswith("projection_"):
                obj[key] = source[key]
        obj["postern_refinement"] = TAG
        obj["part_name"] = "Lower timber ladder" if node == "building-039" else "Upper timber ladder"
        obj["rung_count"] = count
        obj["refinement_note"] = "Open timber ladder audited against covered artwork; hidden timber uses neutral fallback."
        for old in sources:
            old.hide_render = old.hide_viewport = True
            old["postern_refinement_baseline"] = TAG
        results.append({"source_node": node, "rungs": count, **bad})
    bpy.context.view_layer.update()
    return {"reused": False, "ladders": results, "parapet": parapet}
