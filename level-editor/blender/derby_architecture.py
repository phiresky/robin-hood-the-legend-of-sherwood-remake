"""Rebuild audited straight parapets with painted crenellations as real geometry.

The hidden source remains available for revision. Projected UVs are transferred
per surface orientation; run reproject_map after all geometry edits for visibility.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector


def crenellate(source, top_corners, gaps, depth):
    """Extrude a stepped wall profile through its measured thickness.

    top_corners are outer start/end and inner start/end source vertex indices;
    gaps are absolute reference-image x intervals measured at the parapet.
    """
    if source.get('architecture_refined'):
        return {'object': source.name, 'skipped': 'already refined'}
    world = source.matrix_world
    outer_a, outer_b, inner_a, inner_b = [world @ source.data.vertices[i].co for i in top_corners]
    if outer_a.x >= outer_b.x:
        raise ValueError('Parapet endpoints must run left to right')
    bottom = min((world @ v.co).z for v in source.data.vertices)
    profile = [(0, 0)]
    for left, right in gaps:
        a, b = [(x - outer_a.x) / (outer_b.x - outer_a.x) for x in (left, right)]
        if not 0 < a < b < 1 or a < profile[-1][0]:
            raise ValueError('Notches must lie inside the wall in increasing order')
        profile.extend([(a, 0), (a, depth), (b, depth), (b, 0)])
    profile.append((1, 0))
    vertices = []
    for start, end in ((outer_a, outer_b), (inner_a, inner_b)):
        vertices.append(Vector((start.x, start.y, bottom)))
        for t, drop in profile:
            p = start.lerp(end, t)
            p.z -= drop
            vertices.append(p)
        vertices.append(Vector((end.x, end.y, bottom)))
    n = len(profile) + 2
    faces = [tuple(range(n)), tuple(range(n, 2*n))]
    faces.extend((i, (i+1)%n, (i+1)%n+n, i+n) for i in range(n))
    mesh = bpy.data.meshes.new(source.name + ' / crenellated mesh')
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad_edges or bad_faces:
        raise RuntimeError('Invalid crenellated wall topology')
    sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))
    def project(p):
        return Vector((p.x, -p.y*sin-p.z*cos, 1))
    transfers = []
    normal_matrix = world.to_3x3().inverted().transposed()
    for p in source.data.polygons:
        if len(p.vertices) != 3:
            continue
        coordinates = [project(world @ source.data.vertices[i].co) for i in p.vertices]
        mat = Matrix(coordinates).transposed()
        if abs(mat.determinant()) < 1e-7:
            continue
        transfers.append(((normal_matrix @ p.normal).normalized(), mat.inverted(),
                          [source.data.uv_layers.active.data[i].uv.copy() for i in p.loop_indices]))
    view_direction = Vector((0, -cos, sin))
    donor = max(transfers, key=lambda t: t[0].dot(view_direction))
    top = max(v.z for v in vertices)
    layer = mesh.uv_layers.new(name='Projected parapet atlas')
    for p in mesh.polygons:
        normal, inverse, uvs = max(transfers, key=lambda t: t[0].dot(p.normal))
        zs = [mesh.vertices[i].co.z for i in p.vertices]
        reveal = p.index >= 2 and min(zs) > bottom + 1 and min(zs) < top - 2
        if reveal:
            normal, inverse, uvs = donor
        for i in p.loop_indices:
            point = mesh.vertices[mesh.loops[i].vertex_index].co
            if reveal:
                # New cut surfaces have no source pixels: sample a small inset
                # stone patch below this notch, never extrapolate outside its atlas.
                sample = p.center + (point - p.center) * 0.1
                sample.z = top - depth - 24 + (point.z - p.center.z) * 0.1
                weights = inverse @ project(sample)
                weights = Vector(tuple(max(0.02, min(0.96, w)) for w in weights))
                weights /= sum(weights)
            else:
                weights = inverse @ project(point)
            layer.data[i].uv = sum((uvs[k] * weights[k] for k in range(3)), Vector((0, 0)))
    for material in source.data.materials:
        mesh.materials.append(material)
    obj = bpy.data.objects.new(source.name + ' / modeled crenellations', mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in ('source_obstacle', 'source_node', 'asset_group', 'asset_name', 'part_name'):
        obj[key] = source[key]
    obj['crenellation_notches'] = len(gaps)
    obj['todo'] = 'Concealed cut reveals use an inset stone donor; reproject visible surfaces after geometry edits.'
    source['architecture_refined'] = True
    source.hide_render = True
    source.hide_set(True)
    return {'object': obj.name, 'notches': len(gaps), 'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces}


def refine():
    bpy.context.view_layer.update()
    working = bpy.data.collections['Derby Working']
    report = []
    # Two clear open crenels between the three painted merlons on each parapet.
    for source_id, gaps in (
        ('building-174', ((1050, 1058), (1079, 1088))),
        ('building-138', ((460, 471), (495, 506))),
    ):
        source = next(o for o in working.objects if o.get('source_node') == source_id and not o.get('crenellation_notches'))
        report.append(crenellate(source, (18, 19, 17, 16), gaps, 27))
    return report
