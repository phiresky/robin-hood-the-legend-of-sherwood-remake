"""A closed south-curtain lean-to with a thick roof and hollow chimney.

The narrow second volume is a chimney, not a shelter body. The roof is visible
above the curtain; the painting does not establish open timber supports or an
interior room plan. Concealed walls therefore retain their existing fallback.
"""
import importlib.util
from pathlib import Path
import bpy
from mathutils import Matrix, Vector

TAG = "south_curtain_lean_to_refinement"
RECIPE = "south-curtain-lean-to-roof-and-hollow-flue-v1"
IDS = (67, 68)


def _builder():
    # Share the checked projected-UV, fallback-material and closed-mesh builder
    # with the cottage recipe; the named asset and dimensions remain separate.
    path = Path(__file__).with_name('derby_asset_lower_southeast_cottage.py')
    spec = importlib.util.spec_from_file_location('derby_cottage_mesh_builder', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module._mesh, module._prism


def _chimney(source, mesh_builder, side_mappings=(4, 2, 0, 6)):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    outer = [world[i].copy() for i in (16, 17, 18, 19)]
    center = sum(outer, Vector()) / 4
    inside = [center.lerp(p, .55) for p in outer]
    low_inside = [p - Vector((0, 0, 8)) for p in inside]
    bottom = [Vector((p.x, p.y, 0)) for p in outer]
    vertices = outer + inside + low_inside + bottom
    faces = [(15, 14, 13, 12), (8, 9, 10, 11)]
    mappings = [8, 8]
    for i, side in enumerate(side_mappings):
        j = (i + 1) % 4
        faces += [(i, j, j + 12, i + 12), (i, i + 4, j + 4, j),
                  (i + 4, i + 8, j + 8, j + 4)]
        mappings += [side, 8, 8]
    return mesh_builder(source, vertices, faces, mappings,
                        'South curtain lean-to / hollow masonry chimney')


def _roof(source, prism_builder):
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    top = [world[i].copy() for i in (16, 17, 18, 19)]
    underside = [p - Vector((0, 0, 2.5)) for p in top]
    bottom = [Vector((p.x, p.y, 0)) for p in top]
    return [('roof with solid eave thickness', prism_builder(source, top, underside,
             [8, 8, 8, 8, 8, 8], 'South curtain lean-to / roof slab')),
            ('closed supporting wall shell', prism_builder(source, underside, bottom,
             [8, 8, 2, 0, 6, 4], 'South curtain lean-to / supporting walls'))]


def refine():
    collection = bpy.data.collections['Derby Working']
    existing = [o for o in collection.objects if not o.hide_render and o.get(TAG) == RECIPE]
    if existing:
        counts = {number: sum(o.get('source_node') == f'building-{number:03}' for o in existing) for number in IDS}
        if len(existing) != 3 or counts != {67: 1, 68: 2}:
            raise ValueError('Incomplete south-curtain lean-to refinement')
        return {'status': 'existing', 'objects': [o.name for o in existing]}
    bpy.context.view_layer.update()
    mesh_builder, prism_builder = _builder()
    report = []
    for number in IDS:
        candidates = [o for o in collection.objects if o.type == 'MESH' and not o.hide_render
                      and o.get('source_node') == f'building-{number:03}']
        if len(candidates) != 1:
            raise ValueError(f'Expected one lean-to source {number}')
        source = candidates[0]
        pieces = [('hollow masonry chimney', _chimney(source, mesh_builder))] if number == 67 else _roof(source, prism_builder)
        for label, (mesh, defects) in pieces:
            obj = bpy.data.objects.new('East Bailey South Curtain Lean-to / ' + label, mesh)
            collection.objects.link(obj)
            obj.parent = source.parent
            bpy.context.view_layer.update()
            obj.matrix_world = Matrix.Identity(4)
            for key, value in source.items():
                obj[key] = value
            obj[TAG] = RECIPE
            obj['refinement_recipe'] = RECIPE
            obj['asset_name'] = 'East Bailey South Curtain Lean-to'
            obj['part_name'] = 'Masonry chimney' if number == 67 else 'Lean-to roof and walls'
            report.append({'source_node': source['source_node'], 'piece': label,
                           'faces': len(mesh.polygons), 'validation': defects})
        source.hide_render = source.hide_viewport = True
    bpy.context.view_layer.update()
    return {'status': 'created', 'objects': report,
            'audit': {'067': 'Masonry chimney with eight-unit flue recess and retained cap silhouette',
                      '068': 'Monopitch roof separated from closed wall shell; two-and-a-half-unit eave thickness'},
            'catalog_correction': {'asset': 'East Bailey South Curtain Lean-to',
                                   '067': 'Masonry chimney', '068': 'Lean-to roof and walls'},
            'remaining': 'Hidden walls retain imperfect curtain-contaminated fallback atlas; open supports and interior layout are not established by the artwork',
            'uncertainty': 'Roof and chimney recess thicknesses are authored estimates'}


if __name__ == '__main__':
    result = refine()
