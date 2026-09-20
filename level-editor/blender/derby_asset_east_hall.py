"""East Hall's painted battlements as bounded, independently reviewable cuts.

Keep source collision shells and furniture untouched. This recipe only replaces
five exterior render meshes, retaining their source IDs and atlas fallbacks.
The authored gaps use reference-image x coordinates, not evenly spaced guesses.
"""
import math
import runpy
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = 'derby-east-hall-battlements-v1'

# Outer start/end, matching inner endpoints, source-image gap extents, depth.
RECIPES = {
    'building-194': [(18, 19, 17, 16, [(1303, 1316)], 27)],
    'building-183': [
        (4, 6, 12, 13, [(1349, 1359), (1371, 1381), (1393, 1403)], 27),
        (0, 2, 18, 19, [(1433, 1442), (1451, 1462), (1474, 1485), (1497, 1507)], 27),
    ],
    'building-185': [
        (36, 43, 39, 40, [(1380, 1392), (1410, 1422), (1440, 1452),
                         (1501, 1512), (1531, 1543), (1562, 1574), (1592, 1604)], 32),
    ],
    'building-189': [(17, 18, 16, 19, [(1531, 1543)], 27)],
    'building-198': [
        (6, 7, 28, 30, [(1247, 1263)], 36),
        (10, 11, 24, 26, [(1291, 1309)], 36),
        (12, 14, 22, 23, [(1291, 1308)], 36),
    ],
}


def _refine(source, recipes):
    seam_tolerance = 1.2 if source.get('source_node') == 'building-185' else .6
    subtract = runpy.run_path(str(Path(__file__).with_name('derby_east_hall.py')))['_subtract']
    world = source.matrix_world
    vertices = [world @ v.co for v in source.data.vertices]
    uv = next((layer for layer in source.data.uv_layers if layer.active_render), source.data.uv_layers.active)
    fallback = source.data.attributes.get('reprojection_fallback_material')
    faces = [([(vertices[source.data.loops[i].vertex_index].copy(), uv.data[i].uv.copy())
               for i in face.loop_indices],
              fallback.data[face.index].value if fallback else face.material_index)
             for face in source.data.polygons]
    donors = list(faces)
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def projected(p):
        return Vector((p.x, -p.y*sine-p.z*cosine))

    def donor_uv(point, anchor):
        choices = []
        for polygon, material in donors:
            if len(polygon) != 3:
                continue
            a, b, c = [projected(p) for p, _ in polygon]
            basis = Matrix(((b.x-a.x, c.x-a.x), (b.y-a.y, c.y-a.y)))
            if abs(basis.determinant()) < .01:
                continue
            weights = basis.inverted() @ (projected(anchor)-a)
            outside = max(0, -weights.x, -weights.y, weights.x+weights.y-1)
            score = outside*1e7 + ((sum((p for p, _ in polygon), Vector())/3)-anchor).length_squared
            # Concealed reveals have no source observation. Keep their donor
            # inside its atlas triangle, never extrapolate into a neighboring asset.
            weights = basis.inverted() @ (projected(point)-a)
            bary = Vector((1-weights.x-weights.y, weights.x, weights.y))
            bary = Vector(tuple(max(.02, min(.96, w)) for w in bary))
            bary /= sum(bary)
            value = sum((polygon[i][1]*bary[i] for i in range(3)), Vector((0, 0)))
            choices.append((score, value, material))
        if not choices:
            raise ValueError('No valid fallback donor')
        _, value, material = min(choices, key=lambda item: item[0])
        return value, material

    bottoms = []
    count = 0
    for oa, ob, ia, ib, gaps, depth in recipes:
        a, b, inner_a, inner_b = [vertices[i].copy() for i in (oa, ob, ia, ib)]
        tangent = Vector((b.x-a.x, b.y-a.y, 0)).normalized()
        inward = Vector((-tangent.y, tangent.x, 0))
        if inward.dot(inner_a+inner_b-a-b) < 0:
            inward.negate()
        for left, right in gaps:
            if not a.x < left < right < b.x:
                raise ValueError('Gap falls outside audited segment')
            start, end = [a.lerp(b, (x-a.x)/(b.x-a.x)) for x in (left, right)]
            inside_start, inside_end = [inner_a.lerp(inner_b, (tangent.dot(p)-tangent.dot(inner_a))/tangent.dot(inner_b-inner_a)) for p in (start, end)]
            top = max(a.z, b.z)
            bottom = top-depth
            bottoms.append(bottom)
            planes = [(tangent, tangent.dot(end)), (-tangent, -tangent.dot(start)),
                      (inward, max(inward.dot(inner_a), inward.dot(inner_b))+1),
                      (-inward, -min(inward.dot(a), inward.dot(b))+1),
                      (Vector((0, 0, -1)), -bottom)]
            faces = [(piece, material) for polygon, material in faces for piece in subtract(polygon, planes)]
            low = [Vector((p.x, p.y, bottom)) for p in (start, end, inside_end, inside_start)]
            high = [p.copy() for p in (start, end, inside_end, inside_start)]
            for cap in (low, [low[0], high[0], high[3], low[3]], [low[1], low[2], high[2], high[1]]):
                anchor = sum(cap, Vector())/4
                anchor.z = bottom-12
                samples = [donor_uv(anchor+(p-sum(cap, Vector())/4)*.08, anchor) for p in cap]
                faces.append(([(p, sample[0]) for p, sample in zip(cap, samples)], samples[0][1]))
            count += 1
    mesh = bpy.data.meshes.new(source.name+' / open battlements')
    points, polygons, uvs, materials = [], [], [], []
    for polygon, material in faces:
        clean = []
        for point, value in polygon:
            if not clean or (point-clean[-1][0]).length > 1e-6:
                clean.append((point, value))
        if len(clean)>2 and (clean[0][0]-clean[-1][0]).length < 1e-6:
            clean.pop()
        if len(clean)<3 or sum((clean[i][0]-clean[0][0]).cross(clean[i+1][0]-clean[0][0]).length for i in range(1,len(clean)-1)) < 1e-7:
            continue
        polygons.append(tuple(range(len(points), len(points)+len(clean))))
        points.extend(p for p, _ in clean)
        uvs.extend(value for _, value in clean)
        materials.append(material)
    mesh.from_pydata(points, [], polygons)
    for material in source.data.materials:
        mesh.materials.append(material)
    layer = mesh.uv_layers.new(name=uv.name)
    layer.active_render = True
    for loop, value in zip(layer.data, uvs):
        loop.uv = value
    for face, material in zip(mesh.polygons, materials):
        face.material_index = material
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=seam_tolerance)
    points = list(bm.verts)
    for edge in list(bm.edges):
        a, b = edge.verts
        delta = b.co-a.co
        if delta.length_squared < 1e-10:
            continue
        cuts = []
        for vertex in points:
            if vertex in (a, b):
                continue
            fraction = (vertex.co-a.co).dot(delta)/delta.length_squared
            if .00001 < fraction < .99999 and (a.co+fraction*delta-vertex.co).length < seam_tolerance:
                cuts.append(fraction)
        previous, current = 0., a
        for fraction in sorted(set(round(t, 8) for t in cuts)):
            _, inserted = bmesh.utils.edge_split(edge, current, (fraction-previous)/(1-previous))
            previous, current = fraction, inserted
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=seam_tolerance)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad = sum(face.calc_area() < 1e-7 for face in bm.faces)
    def inherited_edge(edge):
        for polygon, _ in donors:
            for (a, _), (b, _) in zip(polygon, polygon[1:]+polygon[:1]):
                delta = b-a
                if delta.length_squared < 1e-10:
                    continue
                if all((a+max(0,min(1,(v.co-a).dot(delta)/delta.length_squared))*delta-v.co).length < seam_tolerance for v in edge.verts):
                    return True
        return False
    open_cuts = sum(not edge.is_manifold and not inherited_edge(edge) and all(v.co.z >= min(bottoms)-.01 for v in edge.verts) for edge in bm.edges)
    bm.to_mesh(mesh)
    bm.free()
    if bad or open_cuts:
        bpy.data.meshes.remove(mesh)
        raise ValueError(f'{source.name}: {bad} degenerate faces; {open_cuts} open cut edges')
    replacement = bpy.data.objects.new(source.name+' / open battlements', mesh)
    bpy.data.collections['Derby Working'].objects.link(replacement)
    replacement.parent = source.parent
    replacement.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        replacement[key] = source[key]
    replacement['refinement_recipe'] = TAG
    replacement['crenellation_notches'] = count
    replacement['todo'] = 'Hidden reveals retain bounded stone donor texture; concealed facade detail needs additional reference.'
    source.hide_render = True
    source.hide_set(True)
    source['replaced_by'] = replacement.name
    bpy.context.view_layer.update()
    return {'source_node': replacement['source_node'], 'notches':count,
            'faces': len(mesh.polygons), 'degenerate_faces':bad, 'open_cut_edges':open_cuts}


def refine():
    bpy.context.view_layer.update()
    working = bpy.data.collections['Derby Working']
    result = []
    for node, recipes in RECIPES.items():
        visible = [o for o in working.all_objects if o.type=='MESH' and o.get('source_node')==node and not o.hide_render]
        if len(visible) != 1:
            raise ValueError(f'Expected one visible {node}')
        if visible[0].get('refinement_recipe') == TAG:
            result.append({'source_node':node, 'status':'already-refined'})
        else:
            result.append(_refine(visible[0], recipes))
    return result
