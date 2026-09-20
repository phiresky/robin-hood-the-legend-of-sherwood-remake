"""Fit the east hall stair to its thirteen painted treads and repair side UVs.

Run ``refine()`` before ``reproject_layers``. The source ramp is retained hidden
as a reversible reference; collision provenance stays on building-081.
"""
import math

import bpy
import bmesh
from mathutils import Matrix, Vector


def refine():
    collection = bpy.data.collections['Derby Working']
    source = next(o for o in collection.objects
                  if o.get('source_node') == 'building-081' and not o.get('step_count'))
    stair = next(o for o in collection.objects
                 if o.get('source_node') == 'building-081' and o.get('step_count'))
    if stair.get('stair_texture_revision') == 1:
        return {'object': stair.name, 'already_applied': True, 'steps': 13,
                'trough': refine_trough()}
    bpy.context.view_layer.update()
    points = sorted((source.matrix_world @ source.data.vertices[i].co
                     for i in (12, 13, 14, 15)), key=lambda p: p.z)
    low = sorted(points[:2], key=lambda p: p.x)
    high = sorted(points[2:], key=lambda p: p.x)
    base = min(p.z for p in low)
    top = sum(p.z for p in high) / 2
    count = 13
    # Audited bright front-edge transitions, measured across the middle 70% of
    # the painted flight. These are source-screen distances from top to bottom.
    front_edges = (.045, .127, .210, .278, .374, .437, .513,
                   .596, .664, .745, .810, .877, .952)
    sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
    width = high[1] - high[0]
    screen_slope = -width.y * sin / width.x

    def intercept(p):
        return -p.y * sin - p.z * cos - screen_slope * p.x

    low_center = sum(low, Vector()) / 2
    high_center = sum(high, Vector()) / 2
    q_low, q_high = intercept(low_center), intercept(high_center)
    heights = []
    for i, edge in enumerate(reversed(front_edges)):
        p = low_center.lerp(high_center, i / count)
        p.z = 0
        q_target = q_high + edge * (q_low - q_high)
        heights.append((intercept(p) - q_target) / cos)
    heights[-1] = top
    if not all(a < b for a, b in zip([base] + heights, heights)):
        raise ValueError('Painted stair edges do not produce ascending steps')
    profile = [(0, base)]
    for i, height in enumerate(heights):
        profile.extend([(i / count, height), ((i + 1) / count, height)])
    profile.append((1, base))
    vertices = []
    for start, end in zip(low, high):
        for t, z in profile:
            point = start.lerp(end, t)
            point.z = z
            vertices.append(point)
    n = len(profile)
    faces = [tuple(range(n)), tuple(range(n, n * 2))]
    faces += [(i, (i + 1) % n, (i + 1) % n + n, i + n) for i in range(n)]
    mesh = bpy.data.meshes.new(stair.name + ' / fitted thirteen treads')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad_edges or bad_faces:
        raise ValueError('Invalid fitted staircase mesh')
    for material in source.data.materials:
        mesh.materials.append(material)
    uv = mesh.uv_layers.new(name='Stair fallback atlas')
    source_uv = source.data.uv_layers.active
    normal_matrix = source.matrix_world.to_3x3().inverted().transposed()
    fallback = source.data.attributes.get('reprojection_fallback_material')
    # Vertical side walls use the matching original wall atlas, never a
    # projection extrapolated from the striped ramp-top triangle.
    for face in mesh.polygons:
        candidate_ids = (0, 1, 2, 3, 4, 5) if face.index in (0, 1, 28, 29) else (6, 7)
        candidate = max((source.data.polygons[i] for i in candidate_ids),
                        key=lambda p: (round((normal_matrix @ p.normal).normalized().dot(face.normal), 3), p.area))
        corners = [source.matrix_world @ source.data.vertices[i].co for i in candidate.vertices]
        origin = corners[0]
        axis_a, axis_b = corners[1] - origin, corners[2] - origin
        normal = axis_a.cross(axis_b).normalized()
        inverse = Matrix((axis_a, axis_b, normal)).transposed().inverted()
        source_coords = [source_uv.data[i].uv.copy() for i in candidate.loop_indices]
        screen_inverse = Matrix([Vector((p.x, -p.y * sin - p.z * cos, 1))
                                 for p in corners]).transposed().inverted() if candidate.index in (6, 7) else None
        for li in face.loop_indices:
            p = mesh.vertices[mesh.loops[li].vertex_index].co
            if screen_inverse is not None:
                weights = screen_inverse @ Vector((p.x, -p.y * sin - p.z * cos, 1))
                uv.data[li].uv = sum((source_coords[i] * weights[i] for i in range(3)), Vector((0, 0)))
            else:
                a, b, _ = inverse @ (p - origin)
                uv.data[li].uv = source_coords[0] + a * (source_coords[1] - source_coords[0]) + b * (source_coords[2] - source_coords[0])
        face.material_index = fallback.data[candidate.index].value if fallback else candidate.material_index
    old_mesh = stair.data
    stair.data = mesh
    stair.matrix_world = Matrix.Identity(4)
    stair['step_count'] = count
    stair['stair_texture_revision'] = 1
    stair['todo'] = 'Concealed rear and underside use the retained masonry fallback.'
    source.hide_render = True
    source.hide_set(True)
    source['replaced_by'] = stair.name
    bpy.context.view_layer.update()
    if old_mesh.users == 0:
        bpy.data.meshes.remove(old_mesh)
    return {'object': stair.name, 'steps': count, 'heights': heights,
            'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces,
            'support_hidden': source.name, 'trough': refine_trough()}


def refine_trough():
    """Give the painted stone trough its open basin, preserving part provenance."""
    obj = next(o for o in bpy.data.collections['Derby Working'].objects
               if o.get('source_node') == 'building-106')
    if obj.get('trough_revision') == 1:
        return {'object': obj.name, 'already_applied': True}
    source = obj.data
    corners = [obj.matrix_world @ source.vertices[i].co for i in (16, 17, 18, 19)]
    center = sum(corners, Vector()) / 4
    axis_a = (corners[1] - corners[0]).normalized()
    axis_b = (corners[3] - corners[0]).normalized()
    half_a = (corners[1] - corners[0]).length / 2
    half_b = (corners[3] - corners[0]).length / 2
    bottom = min((obj.matrix_world @ v.co).z for v in source.vertices)
    vertices = []
    for inset, z in ((0, bottom), (0, center.z), (2.0, center.z), (2.0, center.z - 7)):
        for corner in corners:
            d = corner - center
            p = center + axis_a * (d.dot(axis_a) * (half_a-inset)/half_a) + axis_b * (d.dot(axis_b) * (half_b-inset)/half_b)
            p.z = z
            vertices.append(p)
    faces = [(3, 2, 1, 0), (12, 13, 14, 15)]
    for i in range(4):
        j = (i+1) % 4
        faces.extend(((i,j,j+4,i+4), (i+4,j+4,j+8,i+8), (i+8,j+8,j+12,i+12)))
    mesh = bpy.data.meshes.new(obj['asset_name'] + ' / Stone trough basin')
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    if any(not e.is_manifold for e in bm.edges) or any(f.calc_area() < 1e-8 for f in bm.faces):
        raise ValueError('Invalid stone trough mesh')
    bm.to_mesh(mesh)
    bm.free()
    for material in source.materials:
        mesh.materials.append(material)
    uv = mesh.uv_layers.new(name='Trough fallback atlas')
    source_uv = source.uv_layers.active
    fallback = source.attributes.get('reprojection_fallback_material')
    normal_matrix = obj.matrix_world.to_3x3().inverted().transposed()
    for face in mesh.polygons:
        candidate = max(source.polygons,
                        key=lambda p: (round((normal_matrix @ p.normal).normalized().dot(face.normal), 3), p.area))
        p0,p1,p2 = [obj.matrix_world @ source.vertices[i].co for i in candidate.vertices]
        a,b = p1-p0,p2-p0
        inverse = Matrix((a,b,a.cross(b).normalized())).transposed().inverted()
        coords = [source_uv.data[i].uv.copy() for i in candidate.loop_indices]
        for li in face.loop_indices:
            weights = inverse @ (mesh.vertices[mesh.loops[li].vertex_index].co-p0)
            uv.data[li].uv = coords[0]+weights.x*(coords[1]-coords[0])+weights.y*(coords[2]-coords[0])
        face.material_index = fallback.data[candidate.index].value if fallback else candidate.material_index
    obj.data = mesh
    obj.matrix_world = Matrix.Identity(4)
    obj['part_name'] = 'Stone trough'
    obj.name = obj['asset_name'] + ' / Stone trough'
    obj['trough_revision'] = 1
    if source.users == 0:
        bpy.data.meshes.remove(source)
    return {'object': obj.name, 'source_node': obj['source_node'], 'basin_depth': 7,
            'rim_width': 2, 'nonmanifold_edges': 0}
