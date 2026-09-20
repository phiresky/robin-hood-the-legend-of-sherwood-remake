"""Repair concealed stair cheeks independently of painted tread projection."""
import bpy
from mathutils import Vector
from derby_asset_lower_east_curtain import _stone_donor


def refine():
    """Preserve all geometry and visible tread UVs; use wall masonry on cheeks."""
    stair = next(o for o in bpy.data.collections['Derby Working'].objects
                 if o.get('source_node') == 'building-012' and o.get('step_count'))
    mesh = stair.data
    name = 'Lower east curtain / stair cheek masonry'
    material = bpy.data.materials.get(name)
    if material is None:
        material = _stone_donor().copy()
        material.name = name
    material['projection_preserve'] = True
    layer = mesh.uv_layers.get('Stair cheek masonry') or mesh.uv_layers.new(name='Stair cheek masonry')
    for node in material.node_tree.nodes:
        if node.type == 'UVMAP':
            node.uv_map = layer.name
    if material.name not in mesh.materials:
        mesh.materials.append(material)
    index = mesh.materials.find(material.name)
    fallback = mesh.attributes.get('reprojection_fallback_material')
    if fallback is None:
        fallback = mesh.attributes.new('reprojection_fallback_material', 'INT', 'FACE')
        for face in mesh.polygons:
            fallback.data[face.index].value = face.material_index
    changed = []
    for face in mesh.polygons:
        # These profile n-gons are the two full-height cheeks, not risers.
        if len(face.vertices) < stair['step_count'] * 2 or abs(face.normal.z) > .01:
            continue
        normal = (stair.matrix_world.to_3x3().inverted().transposed() @ face.normal).normalized()
        along = Vector((0, 0, 1)).cross(normal).normalized()
        for li in face.loop_indices:
            point = stair.matrix_world @ mesh.vertices[mesh.loops[li].vertex_index].co
            layer.data[li].uv = (point.dot(along) / 80, point.z / 45)
        face.material_index = index
        fallback.data[face.index].value = index
        changed.append(face.index)
    if len(changed) != 2:
        raise ValueError(f'Expected two stair cheeks, found {changed}')
    stair['stair_cheek_revision'] = 1
    stair['todo'] = 'Fine-fit tread spacing against painted steps; hidden cheeks use masonry donor.'
    return {'object': stair.name, 'source_node': stair['source_node'],
            'side_faces': changed, 'geometry_changed': False,
            'material': material.name, 'uv_layer': layer.name}
