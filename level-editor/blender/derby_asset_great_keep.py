"""Great Keep roof-shell and interior stair corrections; run before reprojection.

Roof obstacles contain vertically extruded columns below their tile surfaces.
Those columns are collision reconstruction artifacts, not tower masonry. Retain
them hidden and construct thin roof panels from the measured eaves and apex.
"""
from pathlib import Path
import json
import math
import runpy
import bpy
import bmesh
from mathutils import Matrix, Vector


ASSET = 'derby-great-keep'
RECIPE = 'great-keep-roof-interior-v1'
BATTLEMENT_RECIPE = 'great-keep-battlements-v1'

# Source-image measurements: paired outer/inner top edges, gap x intervals,
# vertical depth. Smooth turret rings and gallery walls have no painted crenels.
BATTLEMENTS = {
    131: [(4, 11, 16, 15, [(222,230),(240,249),(258,267),(276,283)], 27),
          (4, 0, 16, 23, [(215,224)], 27),
          (0, 2, 23, 27, [(244,262)], 27)],
    132: [(0, 2, 23, 24, [(368,380),(399,410)], 27),
          (82, 78, 32, 41, [(463,480)], 27),
          (41, 42, 78, 74, [(497,511)], 27),
          (52, 61, 66, 65, [(507,515)], 27)],
    133: [(4, 0, 11, 15, [(289,299)], 27)],
    153: [(4, 0, 10, 2, [(775,789),(805,821),(835,852)], 27)],
    158: [(0, 7, 20, 16, [(770,780),(789,798)], 27),
          (0, 2, 20, 24, [(761.2,762.7)], 27),
          (7, 11, 16, 12, [(819,834),(850,866)], 27)],
    164: [(16, 23, 28, 27, [(923,932),(944,952),(964,970)], 27),
          (8, 4, 35, 39, [(939,953)], 27),
          (4, 0, 39, 43, [(983,993),(1012,1023)], 27),
          (0, 2, 43, 47, [(1052,1064),(1076,1087),(1100,1108)], 27)],
}


def _source(number):
    matches = [o for o in bpy.data.collections['Derby Working'].objects
               if o.get('source_node') == f'building-{number:03}'
               and not o.get('great_keep_recipe') and not o.get('step_count')]
    if len(matches) != 1:
        raise ValueError(f'Expected one source for {number}, got {len(matches)}')
    return matches[0]


def _roof_panel(source, triangles, wall_edges, concealed_donor=None):
    mesh = bpy.data.meshes.new(source.name + ' / roof shell')
    vertices, faces, donors = [], [], []
    for triangle in triangles:
        offset = len(vertices)
        vertices.extend(triangle)
        vertices.extend(p - Vector((0, 0, 2)) for p in triangle)
        faces.extend(tuple(offset + i for i in face) for face in
                     ((0, 1, 2), (5, 4, 3), (0, 3, 4, 1),
                      (1, 4, 5, 2), (2, 5, 3, 0)))
        donors.extend([len(source.data.polygons)-1]*5)
    for start, end in wall_edges:
        bottom_a, bottom_b = start.copy(), end.copy()
        bottom_a.z = bottom_b.z = 835.0
        inset = Vector((-(end-start).y, (end-start).x, 0)).normalized()*2
        offset = len(vertices)
        outer = [bottom_a, bottom_b, end, start]
        vertices.extend(outer)
        vertices.extend(p+inset for p in outer)
        faces.extend(tuple(offset+i for i in face) for face in
                     ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)))
        donors.extend([0]*6)
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-8 for f in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad_edges or bad_faces:
        raise ValueError('Roof shell is not closed')
    texture_source = concealed_donor or source
    for material in texture_source.data.materials:
        mesh.materials.append(material)
    # Transfer every UV channel, including explicitly named projection channels.
    # Front roof panels retain their exact source triangle map. Concealed panel
    # and undersides use bounded donor samples until the visibility pass runs.
    for old in source.data.uv_layers:
        layer = mesh.uv_layers.new(name=old.name)
        for face in mesh.polygons:
            donor = source.data.polygons[donors[face.index]]
            points = [source.matrix_world @ source.data.vertices[i].co for i in donor.vertices]
            edge_a, edge_b = points[1]-points[0], points[2]-points[0]
            gram = Matrix(((edge_a.dot(edge_a), edge_a.dot(edge_b)),
                           (edge_b.dot(edge_a), edge_b.dot(edge_b)))).inverted()
            texture_face = texture_source.data.polygons[-1 if donors[face.index] else 0]
            texture_uv = texture_source.data.uv_layers.get(old.name)
            if texture_uv is None:
                raise ValueError(f'Missing roof donor UV channel {old.name}')
            uv = [texture_uv.data[i].uv.copy() for i in texture_face.loop_indices]
            for index in face.loop_indices:
                delta = mesh.vertices[mesh.loops[index].vertex_index].co - points[0]
                a, b = gram @ Vector((delta.dot(edge_a), delta.dot(edge_b)))
                weights = Vector((max(0., 1-a-b), max(0., a), max(0., b)))
                weights /= sum(weights)
                layer.data[index].uv = sum((uv[i]*weights[i] for i in range(3)), Vector((0, 0)))
    mesh.uv_layers.active_index = source.data.uv_layers.active_index
    fallback = texture_source.data.attributes.get('reprojection_fallback_material')
    attr = mesh.attributes.new('reprojection_fallback_material', 'INT', 'FACE')
    for face in mesh.polygons:
        donor = texture_source.data.polygons[-1 if donors[face.index] else 0]
        material_index = fallback.data[donor.index].value if fallback else donor.material_index
        face.material_index = material_index
        attr.data[face.index].value = material_index
    obj = bpy.data.objects.new(source.name + ' / roof shell', mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith('reprojection_'):
            obj[key] = source[key]
    obj['great_keep_recipe'] = RECIPE
    obj['roof_shell_thickness'] = 2.0
    source.hide_render = True
    source.hide_set(True)
    return {'source_node': obj['source_node'], 'panels': len(triangles),
            'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces,
            'minimum_height': min(p.z for p in vertices)}


def _refine_roof_and_stairs():
    bpy.context.view_layer.update()
    working = bpy.data.collections['Derby Working']
    applied = {o.get('source_node') for o in working.objects
               if o.get('great_keep_recipe') == RECIPE}
    expected = {'building-171', 'building-172', 'building-173',
                'building-226', 'building-231'}
    if applied:
        if applied != expected:
            raise ValueError(f'Incomplete Great Keep recipe: {sorted(applied)}')
        return {'asset': ASSET, 'skipped': 'already applied'}
    reports = []
    roofs = [_source(i) for i in (171, 172, 173)]
    top = [[o.matrix_world @ o.data.vertices[i].co
            for i in o.data.polygons[-1].vertices] for o in roofs]
    # Existing three roof facets leave the north face open. Its two eaves are
    # measured on the left/right source facets; use their shared apex midpoint.
    back = [top[1][2], (top[1][1] + top[2][1]) / 2, top[2][0]]
    for i, source in enumerate(roofs):
        edges = [(top[i][0], top[i][2])]
        if i == 2:
            edges.append((back[0], back[2]))
        reports.append(_roof_panel(source, [top[i]] + ([back] if i == 2 else []), edges,
                                   concealed_donor=roofs[0] if i == 2 else None))
    stairs = runpy.run_path(str(Path(__file__).with_name('derby_stair_details.py')))
    for number, faces, count in ((226, (6, 7), 4), (231, (8, 9), 5)):
        source = _source(number)
        result = stairs['add_steps'](source, faces, count)
        obj = bpy.data.objects[result['object']]
        # add_steps only copies editor identifiers; transfer reveal state too.
        for key in source.keys():
            if (key.startswith('reveal_') or key.startswith('sight_patch_')
                    or key in ('projection_layer', 'projection_layer_manifest')):
                obj[key] = source[key]
        obj['great_keep_recipe'] = RECIPE
        result['source_node'] = obj['source_node']
        reports.append(result)
    return {'asset': ASSET, 'changes': reports,
            'remaining': ['Cone curvature still follows four measured planar facets.',
                          'Interior riser counts require final painted-detail alignment.',
                          'Facade recesses and concealed interior surfaces remain.']}


def refine_battlements():
    bpy.context.view_layer.update()
    cut = runpy.run_path(str(Path(__file__).with_name('derby_asset_east_hall.py')))['_refine']
    working = bpy.data.collections['Derby Working']
    report = []
    for number, recipes in BATTLEMENTS.items():
        node = f'building-{number:03}'
        visible = [o for o in working.objects if o.type == 'MESH'
                   and o.get('source_node') == node and not o.hide_render]
        if len(visible) != 1:
            raise ValueError(f'Expected one visible battlement shell for {node}')
        source = visible[0]
        if source.get('refinement_recipe') == BATTLEMENT_RECIPE:
            report.append({'source_node': node, 'status': 'already-refined'})
            continue
        item = cut(source, recipes)
        replacement = bpy.data.objects[source['replaced_by']]
        if number == 164:
            # This short return is vertical in the reference image, so its gap
            # is measured by image y. Rotate only the temporary cutting frame;
            # original UV interpolation and final world coordinates are retained.
            original_points = [source.matrix_world @ source.data.vertices[i].co
                               for i in (16, 12, 28, 35)]
            vertices = [replacement.matrix_world @ v.co for v in replacement.data.vertices]
            indices = [min(range(len(vertices)), key=lambda i: (vertices[i]-point).length_squared)
                       for point in original_points]
            if any((vertices[i]-point).length > .8 for i, point in zip(indices, original_points)):
                raise ValueError('North tower return endpoints changed unexpectedly')
            rotation = Matrix.Rotation(math.pi/2, 4, 'Z')
            previous_matrix = replacement.matrix_world.copy()
            replacement.matrix_world = rotation @ previous_matrix
            bpy.context.view_layer.update()
            sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
            z = original_points[0].z
            intervals = [((239+z*cosine)/sine, (250+z*cosine)/sine)]
            try:
                extra = cut(replacement, [(*indices, intervals, 27)])
            finally:
                replacement.matrix_world = previous_matrix
            final = bpy.data.objects[replacement['replaced_by']]
            inverse = rotation.inverted()
            for vertex in final.data.vertices:
                vertex.co = inverse @ vertex.co
            final.data.update()
            final['crenellation_notches'] = item['notches'] + extra['notches']
            item['notches'] = final['crenellation_notches']
            item['faces'] = len(final.data.polygons)
            replacement = final
        replacement['refinement_recipe'] = BATTLEMENT_RECIPE
        report.append(item)
    return report


def refine():
    """Each additive recipe is independently idempotent across checkpoints."""
    return {'asset': ASSET, 'roof_and_stairs': _refine_roof_and_stairs(),
            'battlements': refine_battlements()}


def audit():
    """Record every catalog component, including unresolved source mesh defects."""
    bpy.context.view_layer.update()
    catalog = json.loads((Path(__file__).parent.parent / 'shared/assets/derby.json').read_text())
    group = next(g for g in catalog['groups'] if g['id'] == ASSET)
    working = bpy.data.collections['Derby Working']
    parts = []
    for part in group['parts']:
        node = f"building-{part['obstacle']:03}"
        visible = [o for o in working.objects if o.type == 'MESH'
                   and o.get('source_node') == node and not o.hide_render]
        if not visible:
            raise ValueError(f'Missing visible component {node}')
        components = []
        for obj in visible:
            if obj.get('asset_group') != ASSET or obj.parent is None:
                raise ValueError(f'Broken ownership: {obj.name}')
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            components.append({'name': obj.name, 'faces': len(bm.faces),
                               'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
                               'degenerate_faces': sum(f.calc_area() < 1e-8 for f in bm.faces),
                               'projection_layer': obj.get('projection_layer'),
                               'refined': bool(obj.get('great_keep_recipe') or obj.get('step_count')
                                               or obj.get('crenellation_notches'))})
            bm.free()
        parts.append({'source_node': node, 'name': part['name'], 'components': components})
    return {'asset': ASSET, 'catalog_part_count': len(parts), 'parts': parts,
            'interpretation': 'Source shell boundary edges are reported, not blindly filled; interior/exterior apertures require artwork review.'}
