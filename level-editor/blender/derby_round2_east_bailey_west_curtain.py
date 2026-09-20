"""Recover courtyard buttress relief against the existing continuous wall planes."""
import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-bailey-west-curtain'
TAG = 'round2-courtyard-buttresses-v1'


def refine():
    collection = bpy.data.collections['Derby Working']
    owned = [o for o in collection.objects if o.type == 'MESH'
             and o.get('asset_group') == ASSET and not o.hide_render]
    existing = [o for o in owned if o.get('round2_curtain_support') == TAG]
    if existing:
        if {o.get('source_node') for o in existing} != {
                'building-102', 'building-103', 'building-104', 'building-105'} or len(existing) != 4:
            raise RuntimeError('Incomplete courtyard support refinement')
        return {'reused': True, 'supports': [o.name for o in existing]}
    by_node = {o.get('source_node'): o for o in owned}
    wall = by_node['building-086']
    wp = [wall.matrix_world @ v.co for v in wall.data.vertices]
    # Front wall segments are measured from the retained continuous masonry.
    # Base elevations follow the visible terrain contact, not the hidden z=0 shell.
    specs = [
        ('building-102', 44, 40, (1063.3, -2639.9), 26, 8, 15, 92, 118),
        ('building-103', 40, 36, (1115.2, -2523.8), 23, 7, 13, 91, 120),
        ('building-104', 40, 36, (1143.2, -2428.8), 24, 7, 15, 91, 120),
        ('building-105', 36, 32, (1178.0, -2324.6), 32, 10, 24, 102, 129),
    ]
    report = []
    for node, ia, ib, target, width, depth, base, shoulder, top in specs:
        owner = by_node[node]
        a, b = wp[ia].copy(), wp[ib].copy()
        a.z = b.z = 0
        tangent = (b - a).normalized()
        normal = Vector((tangent.y, -tangent.x, 0))
        center = a + tangent * ((Vector((*target, 0)) - a).dot(tangent))
        vertices = []
        for z, front_depth in ((base, depth), (shoulder, depth), (top, .05)):
            for along, outward in ((-width / 2, -.25), (width / 2, -.25),
                                    (width / 2, front_depth), (-width / 2, front_depth)):
                p = center + tangent * along + normal * outward
                p.z = z
                vertices.append(owner.matrix_world.inverted() @ p)
        faces = [(3, 2, 1, 0), (8, 9, 10, 11)]
        for k in (0, 4):
            faces.extend((k+i, k+(i+1)%4, k+4+(i+1)%4, k+4+i) for i in range(4))
        mesh = bpy.data.meshes.new(node + ' tapered courtyard support')
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        nonmanifold = sum(not e.is_manifold for e in bm.edges)
        degenerate = sum(f.calc_area() < 1e-8 for f in bm.faces)
        bm.to_mesh(mesh)
        bm.free()
        if nonmanifold or degenerate:
            raise RuntimeError('Invalid buttress geometry for ' + node)
        obj = bpy.data.objects.new(owner.name + ' / tapered courtyard buttress', mesh)
        collection.objects.link(obj)
        obj.parent = owner.parent
        obj.matrix_world = owner.matrix_world.copy()
        for key in owner.keys():
            if not key.startswith(('reprojection', 'projection_', 'source_ownership')):
                obj[key] = owner[key]
        obj['round2_curtain_support'] = TAG
        obj['component_role'] = 'courtyard-buttress'
        mesh.uv_layers.new(name='UVMap')
        if owner.data.materials:
            mesh.materials.append(owner.data.materials[0])
        report.append({'source_node': node, 'object': obj.name,
                       'nonmanifold_edges': nonmanifold, 'degenerate_faces': degenerate,
                       'base_z': base, 'top_z': top, 'outward_depth': depth})
    return {'supports': report, 'retained_existing_parts': 14}
