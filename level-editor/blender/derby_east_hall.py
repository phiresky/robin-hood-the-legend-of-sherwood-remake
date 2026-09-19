"""Audited east-watchtower parapet refinement, run through Blender MCP.

The three visible front embrasures are present in the source painting but the
initial volume has a continuous parapet. Clip only these bounded gaps; retain
all lower wall, roof access, interior and turret geometry. Existing faces retain
interpolated atlas UVs; new reveals use the same reference-camera projection.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = "derby-watchtower-embrasures-v1"


def _split(poly, normal, offset):
    inside, outside = [], []
    for a, b in zip(poly, poly[1:] + poly[:1]):
        da, db = normal.dot(a[0]) - offset, normal.dot(b[0]) - offset
        (inside if da <= 0 else outside).append(a)
        if (da < -1e-8 and db > 1e-8) or (db < -1e-8 and da > 1e-8):
            t = da / (da - db)
            crossing = (a[0].lerp(b[0], t), a[1].lerp(b[1], t))
            inside.append(crossing)
            outside.append(crossing)
    return inside, outside


def _subtract(poly, planes):
    kept = []
    for normal, offset in planes:
        poly, outside = _split(poly, normal, offset)
        if len(outside) >= 3:
            kept.append(outside)
        if len(poly) < 3:
            break
    return kept


def refine():
    """Idempotently replace building-215, leaving the original hidden intact."""
    working = bpy.data.collections["Derby Working"]
    existing = [o for o in working.all_objects if o.get("refinement_recipe") == TAG]
    if existing:
        return {"status": "already-refined", "object": existing[0].name}
    sources = [o for o in working.all_objects if o.type == "MESH"
               and o.get("source_node") == "building-215" and not o.hide_render]
    if len(sources) != 1:
        raise ValueError(f"Expected one visible watchtower parapet; found {len(sources)}")
    source = sources[0]
    bpy.context.view_layer.update()
    verts = [source.matrix_world @ v.co for v in source.data.vertices]
    if len(verts) != 124 or len(source.data.polygons) != 78:
        raise ValueError("Watchtower topology changed; re-audit the vertex recipe")
    layer = source.data.uv_layers.active
    if layer is None:
        raise ValueError("Watchtower source has no atlas UV layer")
    faces = []
    fallback = source.data.attributes.get("reprojection_fallback_material")
    for face in source.data.polygons:
        faces.append(([(verts[source.data.loops[i].vertex_index].copy(),
                        layer.data[i].uv.copy()) for i in face.loop_indices],
                      fallback.data[face.index].value if fallback else face.material_index))
    source_faces = list(faces)
    # Outer/inner corner indices, along-edge gap range, and reveal depth. Each
    # endpoint was checked against the visible parapet in the Day source image.
    recipes = [(102, 103, 105, 104, .23, .74, 24.0),
               (109, 110, 108, 107, .27, .75, 24.0),
               (110, 111, 107, 117, .27, .72, 24.0)]
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def projected(p):
        return Vector((p.x, -p.y * sine - p.z * cosine))

    def uv_for(p, anchor):
        best = None
        q = projected(anchor)
        for polygon, _ in source_faces:
            if len(polygon) != 3:
                continue
            a, b, c = [projected(v[0]) for v in polygon]
            basis = Matrix(((b.x-a.x, c.x-a.x), (b.y-a.y, c.y-a.y)))
            if abs(basis.determinant()) < .01:
                continue
            weights = basis.inverted() @ (q-a)
            # Prefer nearby source triangles in 3D, to avoid another atlas island.
            center = sum((v[0] for v in polygon), Vector()) / 3
            outside = max(0.0, -weights.x, -weights.y, weights.x + weights.y - 1)
            score = outside * 1e8 + (center-anchor).length_squared
            if best is None or score < best[0]:
                weights = basis.inverted() @ (projected(p)-a)
                uv = polygon[0][1] + weights.x * (polygon[1][1]-polygon[0][1]) + weights.y * (polygon[2][1]-polygon[0][1])
                best = (score, uv)
        if best is None:
            raise ValueError("No invertible source projection for notch UV")
        return best[1]

    notch_bottoms = []
    for outer_a, outer_b, inner_a, inner_b, lo, hi, depth in recipes:
        a, b, ia, ib = [verts[i].copy() for i in (outer_a, outer_b, inner_a, inner_b)]
        tangent = Vector((b.x-a.x, b.y-a.y, 0)).normalized()
        inward = Vector((-tangent.y, tangent.x, 0))
        if inward.dot((ia+ib-a-b)/2) < 0:
            inward.negate()
        start, end = a.lerp(b, lo), a.lerp(b, hi)
        top = max(a.z, b.z)
        bottom = top - depth
        notch_bottoms.append(bottom)
        # The tangent cut planes intersect the inner edge at slightly different
        # parameters because the ring thickness changes across each polygon.
        inner_start = ia.lerp(ib, (tangent.dot(start)-tangent.dot(ia))/tangent.dot(ib-ia))
        inner_end = ia.lerp(ib, (tangent.dot(end)-tangent.dot(ia))/tangent.dot(ib-ia))
        planes = [(tangent, tangent.dot(end)), (-tangent, -tangent.dot(start)),
                  (inward, max(inward.dot(ia), inward.dot(ib))+2),
                  (-inward, -min(inward.dot(a), inward.dot(b))+2),
                  (Vector((0,0,-1)), -bottom)]
        faces = [(piece, material) for polygon, material in faces for piece in _subtract(polygon, planes)]
        low = [Vector((p.x,p.y,bottom)) for p in (start,end,inner_end,inner_start)]
        high = [Vector((p.x,p.y,top)) for p in (start,end,inner_end,inner_start)]
        caps = [low, [low[0], high[0], high[3], low[3]],
                [low[1], low[2], high[2], high[1]]]
        # Use one atlas island for each complete reveal rather than switching
        # islands independently at its corners.
        faces.extend(([(p, uv_for(p, sum(cap, Vector())/len(cap))) for p in cap], 0) for cap in caps)
    mesh = bpy.data.meshes.new("East Watchtower — open battlement gaps")
    points, polygons, uv_values, materials = [], [], [], []
    for face, material in faces:
        # Clipping sometimes produces a repeated boundary vertex.
        clean = []
        for point, uv in face:
            if not clean or (clean[-1][0]-point).length > 1e-6:
                clean.append((point,uv))
        if len(clean)>2 and (clean[0][0]-clean[-1][0]).length < 1e-6:
            clean.pop()
        if len(clean)<3:
            continue
        area = sum(((clean[i][0]-clean[0][0]).cross(clean[i+1][0]-clean[0][0]).length for i in range(1,len(clean)-1)))
        if area < 1e-6:
            continue
        polygons.append(tuple(range(len(points),len(points)+len(clean))))
        points.extend(p for p,_ in clean)
        uv_values.extend(uv for _,uv in clean)
        materials.append(material)
    mesh.from_pydata(points, [], polygons)
    for material in source.data.materials:
        mesh.materials.append(material)
    uv_layer = mesh.uv_layers.new(name=layer.name)
    for i, uv in enumerate(uv_values):
        uv_layer.data[i].uv = uv
    for face, material in zip(mesh.polygons, materials):
        face.material_index = material
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    # Exported wall strips are offset by fractions of a source pixel. Join
    # those coincident corners before stitching the newly cut reveal edges.
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.6)
    # Polygon clipping introduces vertices along neighboring unsplit edges.
    # Insert those shared points before welding so each boundary has matching
    # topology; UV interpolation is carried by the edge split operation.
    points = list(bm.verts)
    for edge in list(bm.edges):
        a, b = edge.verts
        delta = b.co-a.co
        length_squared = delta.length_squared
        if length_squared < 1e-10:
            continue
        cuts = []
        for vertex in points:
            if vertex in (a,b):
                continue
            fraction = (vertex.co-a.co).dot(delta)/length_squared
            if .00001 < fraction < .99999 and (a.co+fraction*delta-vertex.co).length < .6:
                cuts.append(fraction)
        previous = 0.0
        current = a
        for fraction in sorted(set(round(t,8) for t in cuts)):
            _, inserted = bmesh.utils.edge_split(edge, current, (fraction-previous)/(1-previous))
            current = inserted
            previous = fraction
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.6)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    nonmanifold_notch_edges = sum(
        not edge.is_manifold and all(v.co.z >= min(notch_bottoms)-.01 for v in edge.verts)
        for edge in bm.edges)
    degenerate_faces = sum(face.calc_area() < 1e-7 for face in bm.faces)
    if nonmanifold_notch_edges or degenerate_faces:
        bm.free()
        bpy.data.meshes.remove(mesh)
        raise ValueError(f"Invalid notch topology: {nonmanifold_notch_edges} open edges, {degenerate_faces} degenerate faces")
    bm.to_mesh(mesh)
    bm.free()
    replacement = bpy.data.objects.new(source.name + " / open embrasures", mesh)
    working.objects.link(replacement)
    replacement.parent = source.parent
    replacement.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        replacement[key] = source[key]
    replacement["refinement_recipe"] = TAG
    replacement["embrasure_count"] = len(recipes)
    replacement["part_name"] = "Tower battlements with open embrasures"
    source.hide_render = True
    source.hide_set(True)
    source["replaced_by"] = replacement.name
    bpy.context.view_layer.update()
    return {"status":"refined", "object":replacement.name, "source_node":"building-215",
            "embrasures":len(recipes), "faces":len(mesh.polygons), "vertices":len(mesh.vertices),
            "nonmanifold_notch_edges":nonmanifold_notch_edges, "degenerate_faces":degenerate_faces}
