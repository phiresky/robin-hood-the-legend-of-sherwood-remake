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
               if o.get('great_keep_recipe') == RECIPE and o.get('asset_group') == ASSET}
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
            'battlements': refine_battlements(), 'round_turret': refine_round_turret(),
            'hanging_turret': refine_hanging_turret()}


def refine_round_turret():
    """Replace the four-sided roof reconstruction with the painted round spire.

    Eaves, peak and drum heights are measured in the reference projection.
    Keep three canonical selection parts, but use matching angular boundaries.
    """
    working = bpy.data.collections['Derby Working']
    tag = 'great-keep-round-turret-v2'
    applied={o.get('source_node') for o in working.objects
             if o.get('great_keep_second_pass') == tag and not o.hide_render}
    if applied:
        if applied != {'building-171','building-172','building-173'}:
            raise ValueError(f'Incomplete round turret: {sorted(applied)}')
        return {'status': 'already-refined'}
    changes = []
    for segment, number in enumerate((171, 172, 173)):
        source = next(o for o in working.objects if o.get('source_node') == f'building-{number:03}'
                      and not o.hide_render)
        vertices, faces = [], []
        def point(angle, radius, z):
            return (1003.1 + math.cos(angle)*radius, -1541.4 + math.sin(angle)*radius, z)
        start = segment * 2*math.pi/3
        # A solid thin roof sector with a curved profile, constant source eave.
        slices, rings = 24, 20
        for inside in (False, True):
            for j in range(rings+1):
                t = j/rings
                radius = .35 + 37.65*(1-t)**1.6
                for i in range(slices+1):
                    vertices.append(point(start+i*2*math.pi/72, radius, 907.7+109*t-(2 if inside else 0)))
        layer = (slices+1)*(rings+1)
        for j in range(rings):
            for i in range(slices):
                a = j*(slices+1)+i
                faces.extend([(a,a+1,a+slices+2,a+slices+1),
                              (a+layer+slices+1,a+layer+slices+2,a+layer+1,a+layer)])
        for j in (0,rings):
            for i in range(slices):
                a=j*(slices+1)+i
                faces.append((a,a+layer,a+layer+1,a+1))
        for i in (0,slices):
            for j in range(rings):
                a=j*(slices+1)+i
                faces.append((a,a+slices+1,a+slices+1+layer,a+layer))
        # Round masonry drum; an arched doorway is open in the front sector.
        for i in range(slices):
            angles=[start+k*2*math.pi/72 for k in (i,i+1)]
            bottoms=[]
            for angle in angles:
                offset=math.atan2(math.sin(angle+math.pi/2),math.cos(angle+math.pi/2))
                x=33*math.sin(offset)
                bottoms.append(875+math.sqrt(max(0,12**2-x*x)) if abs(offset)<math.asin(12/33) else 835)
            base=len(vertices)
            for radius in (33,30):
                for a,z in ((angles[0],bottoms[0]),(angles[1],bottoms[1]),(angles[1],907.7),(angles[0],907.7)):
                    vertices.append(point(a,radius,z))
            faces.extend(tuple(base+k for k in f) for f in ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),(2,6,7,3),(3,7,4,0)))
        mesh=bpy.data.meshes.new(source.name+' / curved round shell')
        mesh.from_pydata(vertices,[],faces)
        bm=bmesh.new();bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        invalid=sum(not e.is_manifold for e in bm.edges)
        degenerate=sum(f.calc_area()<1e-8 for f in bm.faces)
        bm.to_mesh(mesh);bm.free()
        if invalid or degenerate:
            raise ValueError('Invalid turret shell topology')
        neutral=bpy.data.materials.get('Great Keep / unobserved turret')
        if neutral is None:
            neutral=bpy.data.materials.new('Great Keep / unobserved turret')
            neutral.diffuse_color=(.22,.22,.22,1)
        mesh.materials.append(neutral)
        for old in source.data.uv_layers:
            layer_uv=mesh.uv_layers.new(name=old.name)
            for uv in layer_uv.data: uv.uv=(.5,.5)
        obj=bpy.data.objects.new(source.name+' / round turret',mesh)
        working.objects.link(obj);obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
        for key in source.keys():
            if not key.startswith('reprojection_'): obj[key]=source[key]
        obj['great_keep_second_pass']=tag
        obj['projection_min_cosine']=.2
        source.hide_render=True;source.hide_set(True)
        changes.append({'source_node':obj['source_node'],'faces':len(faces),'nonmanifold_edges':invalid,'degenerate_faces':degenerate})
    return {'changes':changes,'radial_segments':72,'profile_rings':20,'roof_thickness':2}


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


def refine_hanging_turret():
    """End the east bartizan at its corbel instead of projecting it to ground."""
    working=bpy.data.collections['Derby Working']
    source=next(o for o in working.objects if o.get('source_node')=='building-175' and not o.hide_render)
    tag='great-keep-hanging-bartizan-v1'
    if source.get('great_keep_bartizan')==tag:return {'status':'already-refined'}
    old=source.data
    vertices,faces,uvs,materials=[],[],[],[]
    names=[u.name for u in old.uv_layers]
    def add(poly,material):
        if len(poly)<3:return
        base=len(vertices);vertices.extend(source.matrix_world.inverted()@p for p,u in poly)
        faces.append(tuple(range(base,base+len(poly))));uvs.append([u for p,u in poly]);materials.append(material)
    # Preserve the measured parapet, including its existing crenels and floor.
    for face in old.polygons:
        poly=[(source.matrix_world@old.vertices[old.loops[i].vertex_index].co,[old.uv_layers[n].data[i].uv.copy() for n in names]) for i in face.loop_indices]
        clipped=[]
        for p,q in zip(poly,poly[1:]+poly[:1]):
            ip,iq=p[0].z>=835,q[0].z>=835
            if ip:clipped.append(p)
            if ip!=iq:
                t=(835-p[0].z)/(q[0].z-p[0].z)
                clipped.append((p[0].lerp(q[0],t),[a.lerp(b,t) for a,b in zip(p[1],q[1])]))
        add(clipped,face.material_index)
    # Profile measurements include the two projecting string courses and the
    # corbel's receding stone courses. The lower tip joins the tower wall.
    profile=[(835,1),(772,1),(768,1.06),(763,1.06),(759,1),
             (706,1),(701,1.07),(696,1.07),(692,.98),
             (684,.87),(676,.72),(668,.55),(660,.38),(651,.18)]
    rings=[];segments=64
    unknown_material=len(old.materials)
    for z,radius in profile:
        center=1128-18*(1-radius)
        rings.append([(Vector((center+30.4*radius*math.cos(i*2*math.pi/segments),
                                -1631+37*radius*math.sin(i*2*math.pi/segments),z)),
                       [Vector((.5,.5)) for n in names]) for i in range(segments)])
    for upper,lower in zip(rings,rings[1:]):
        for i in range(segments):
            j=(i+1)%segments
            add([upper[i],lower[i],lower[j],upper[j]],unknown_material)
    add(list(reversed(rings[-1])),unknown_material)
    add(rings[0],unknown_material)
    mesh=bpy.data.meshes.new(source.name+' / hanging bartizan')
    mesh.from_pydata(vertices,[],faces)
    for mat in old.materials:mesh.materials.append(mat)
    neutral=bpy.data.materials.get('Great Keep / unobserved turret')
    if neutral is None:
        neutral=bpy.data.materials.new('Great Keep / unobserved turret')
        neutral.diffuse_color=(.22,.22,.22,1)
    mesh.materials.append(neutral)
    for channel,name in enumerate(names):
        layer=mesh.uv_layers.new(name=name)
        for face,values in zip(mesh.polygons,uvs):
            for li,value in zip(face.loop_indices,values):layer.data[li].uv=value[channel]
    for face,material in zip(mesh.polygons,materials):face.material_index=material
    source.data=mesh;source['great_keep_bartizan']=tag
    source['projection_min_cosine']=.2
    return {'source_node':'building-175','previous_bottom':0,'new_bottom':651,
            'radial_segments':segments,'profile_rings':len(profile),'faces':len(faces)}
