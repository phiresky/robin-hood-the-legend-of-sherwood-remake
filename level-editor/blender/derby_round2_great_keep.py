"""Measured Great Keep rooftop parapet correction.

The narrow rear crenellated strip belongs to the north tower terrace. Its
collision extrusion previously continued 835 units below its supporting roof,
making an isolated vertical blade visible from the side and rear.
"""
import bpy
import bmesh


def refine():
    matches = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == 'building-174'
               and o.get('asset_group') == 'derby-great-keep']
    if len(matches) != 1:
        raise ValueError('Expected one visible Great Keep rear parapet')
    obj = matches[0]
    before = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
    inverse = obj.matrix_world.inverted()
    changed = 0
    for vertex in obj.data.vertices:
        world = obj.matrix_world @ vertex.co
        if world.z < 835.0:
            world.z = 835.0
            vertex.co = inverse @ world
            changed += 1
    obj.data.update()
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.free()
    if bad_edges or bad_faces:
        raise ValueError('Rear parapet must remain a closed nondegenerate shell')
    obj['round2_keep_recipe'] = 'north-roof-parapet-base-v1'
    return {'asset': 'derby-great-keep', 'changed_nodes': ['building-174'],
            'changed_vertices': changed, 'old_bottom': before, 'new_bottom': 835.0,
            'faces': len(obj.data.polygons), 'nonmanifold_edges': bad_edges,
            'degenerate_faces': bad_faces,
            'evidence': 'Native scenery mask144 and original roof terrace image'}
