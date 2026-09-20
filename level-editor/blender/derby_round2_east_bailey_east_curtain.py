"""Consolidate the east curtain stair cheeks and their overlapping ramp supports."""
import bpy
import bmesh

ASSET = 'derby-east-bailey-east-curtain'
TAG = 'east-curtain-single-stair-support-v1'


def refine():
    owned = [o for o in bpy.data.collections['Derby Working'].objects
             if o.type == 'MESH' and o.get('asset_group') == ASSET]
    result = []
    for node in ('building-077', 'building-078'):
        stair = next(o for o in owned if o.get('source_node') == node and o.get('step_count'))
        support = next(o for o in owned if o.get('source_node') == node and o.get('east_bailey_support'))
        if stair.get('round2_refinement') == TAG:
            continue
        points = [stair.matrix_world @ v.co for v in stair.data.vertices]
        half = len(points) // 2
        ground = min((support.matrix_world @ v.co).z for v in support.data.vertices)
        # Keep the audited tread positions. Extend the elevated stair's cheeks
        # down to its supporting wall base, replacing both intersecting shells
        # with one solid. The lower stair already contains its complete cheek.
        if node == 'building-077':
            profiles = []
            for side in (points[:half], points[half:]):
                bottom = side[0].copy()
                bottom.z = ground
                end = side[-1].copy()
                end.z = ground
                profiles.append([bottom] + side[:-1] + [end])
            n = len(profiles[0])
            inverse = stair.matrix_world.inverted()
            vertices = [inverse @ p for side in profiles for p in side]
            faces = [tuple(range(n)), tuple(range(n, 2*n))]
            faces.extend((i, (i+1)%n, (i+1)%n+n, i+n) for i in range(n))
            mesh = bpy.data.meshes.new('Upper east curtain stair / continuous supported cheek')
            mesh.from_pydata(vertices, [], faces)
            for material in stair.data.materials:
                mesh.materials.append(material)
            bm = bmesh.new()
            bm.from_mesh(mesh)
            bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
            if any(not e.is_manifold for e in bm.edges) or any(f.calc_area() < 1e-8 for f in bm.faces):
                raise ValueError('Consolidated stair must be closed and nondegenerate')
            bm.to_mesh(mesh)
            bm.free()
            stair.data = mesh
            mesh.uv_layers.new(name='UVMap')
        support.hide_render = True
        support.hide_set(True)
        support['replaced_by'] = stair.name
        stair['round2_refinement'] = TAG
        stair['todo'] = 'Hidden masonry detail remains unsupported by source imagery.'
        result.append({'source_node': node, 'support_hidden': support.name,
                       'tread_positions_preserved': True, 'support_base': ground})
    bpy.context.view_layer.update()
    validation = []
    for obj in owned:
        if obj.hide_render:
            continue
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        validation.append({'source_node': obj['source_node'],
                           'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
                           'degenerate_faces': sum(f.calc_area() < 1e-8 for f in bm.faces)})
        bm.free()
    if any(r['nonmanifold_edges'] or r['degenerate_faces'] for r in validation):
        raise ValueError(validation)
    return {'asset': ASSET, 'changes': result, 'validation': validation}
