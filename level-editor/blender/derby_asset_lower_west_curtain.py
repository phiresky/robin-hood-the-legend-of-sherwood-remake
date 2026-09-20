"""Model the lower west curtain's painted battlements as closed masonry.

Audited against the covered Derby reference. The wall footprints and source IDs
stay fixed. Concealed cut surfaces sample nearby masonry; run visibility-aware
reprojection after integrating the resulting geometry.
"""
import math
import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = 'lower-west-curtain-crenels-v2'


def _project(point):
    return Vector((point.x, -point.y * math.sin(math.radians(35))
                   - point.z * math.cos(math.radians(35)), 1))


def _wall(source, cuts):
    points = [source.matrix_world @ v.co for v in source.data.vertices]
    top = max(p.z for p in points)
    bottom = min(p.z for p in points)
    roof = [p for p in source.data.polygons
            if all(abs(points[i].z - top) < .01 for i in p.vertices)]
    if not roof:
        raise ValueError('Missing flat source footprint: ' + source.name)
    bm = bmesh.new()
    vertices = {i: bm.verts.new(points[i]) for f in roof for i in f.vertices}
    for face in roof:
        bm.faces.new([vertices[i] for i in face.vertices])
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.001)
    boundary = [e for e in bm.edges if e.is_boundary]
    extrusion = bmesh.ops.extrude_face_region(bm, geom=list(bm.faces) + boundary)
    lowered = [v for v in extrusion['geom'] if isinstance(v, bmesh.types.BMVert)]
    bmesh.ops.translate(bm, verts=lowered, vec=(0, 0, bottom-top))
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    mesh = bpy.data.meshes.new(source.name + ' / closed battlements')
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(source.name + ' / modeled battlements', mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent = source.parent
    obj.matrix_world = Matrix.Identity(4)
    for key in source.keys():
        obj[key] = source[key]
    for material in source.data.materials:
        mesh.materials.append(material)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    count = 0
    for start, end, axis, intervals in cuts:
        a, b = points[start], points[end]
        direction = (b-a).normalized()
        qa, qb = _project(a), _project(b)
        for left, right in intervals:
            ta, tb = sorted(((left-qa[axis])/(qb[axis]-qa[axis]),
                             (right-qa[axis])/(qb[axis]-qa[axis])))
            if not 0 < ta < tb < 1:
                raise ValueError('Crenel outside audited wall span')
            center = a.lerp(b, (ta+tb)/2)
            center.z = top + 20
            bpy.ops.mesh.primitive_cube_add(size=1, location=center)
            cutter = bpy.context.object
            cutter.name = 'Temporary audited crenel cutter'
            cutter.rotation_euler.z = math.atan2(direction.y, direction.x)
            cutter.dimensions = ((b-a).length*(tb-ta), 42, 92)
            bpy.context.view_layer.update()
            bpy.context.view_layer.objects.active = obj
            modifier = obj.modifiers.new('Audited crenel', 'BOOLEAN')
            modifier.operation = 'DIFFERENCE'
            modifier.solver = 'EXACT'
            modifier.object = cutter
            bpy.ops.object.modifier_apply(modifier=modifier.name)
            bpy.data.objects.remove(cutter, do_unlink=True)
            count += 1
    mesh = obj.data
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-7 for f in bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    if bad_edges or bad_faces:
        raise RuntimeError(f'Invalid battlements: {bad_edges} edges, {bad_faces} faces')
    normal_matrix = source.matrix_world.to_3x3().inverted().transposed()
    donors = []
    fallback = source.data.attributes.get('reprojection_fallback_material')
    for face in source.data.polygons:
        if len(face.vertices) != 3:
            continue
        inverse = Matrix([_project(points[i]) for i in face.vertices]).transposed()
        if abs(inverse.determinant()) < 1e-7:
            continue
        donors.append(((normal_matrix @ face.normal).normalized(), inverse.inverted(),
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices],
                       fallback.data[face.index].value if fallback else face.material_index))
    # Boolean cutters introduce their own empty UV layer and material slot.
    # Remove those before binding the reconstructed atlas coordinates.
    while mesh.uv_layers:
        mesh.uv_layers.remove(mesh.uv_layers[0])
    mesh.materials.clear()
    for material in source.data.materials:
        mesh.materials.append(material)
    uv = mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    material_backup = mesh.attributes.new('reprojection_fallback_material', 'INT', 'FACE')
    # Boolean reveals have no photographic counterpart. Sample masonry below
    # the crenel rather than extrapolating into the atlas's black gutter.
    for face in mesh.polygons:
        donor = max(donors, key=lambda item: item[0].dot(face.normal))
        normal, inverse, coordinates, material = donor
        cut = min(mesh.vertices[i].co.z for i in face.vertices) > top-27.1
        if cut:
            donor = max(donors, key=lambda item: item[0].dot(Vector((0,-.819,.574))))
            normal, inverse, coordinates, material = donor
        face.material_index = material
        material_backup.data[face.index].value = material
        for loop_index in face.loop_indices:
            point = mesh.vertices[mesh.loops[loop_index].vertex_index].co.copy()
            if cut:
                point = face.center + (point-face.center)*.12
                point.z = top-48
            weights = inverse @ _project(point)
            if cut:
                weights = Vector(tuple(max(.02,min(.96,w)) for w in weights))
                weights /= sum(weights)
            uv.data[loop_index].uv = sum((coordinates[k]*weights[k] for k in range(3)), Vector((0,0)))
    obj['lower_west_curtain_refinement'] = TAG
    obj['crenellation_notches'] = count
    source['lower_west_curtain_baseline'] = TAG
    source.hide_render = True
    source.hide_set(True)
    return {'source_node': obj['source_node'], 'object': obj.name,
            'notches': count, 'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces}


def _stair(source):
    """Model the narrow access flight; retain its masonry support underneath."""
    import runpy
    from pathlib import Path
    add_steps = runpy.run_path(str(Path(__file__).with_name('derby_stair_details.py')))['add_steps']
    points=[source.matrix_world@v.co for v in source.data.vertices]
    face=source.data.polygons[9]
    inverse=Matrix([_project(points[i]) for i in face.vertices]).transposed().inverted()
    coords=[source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices]
    mesh=bpy.data.meshes.new('West curtain stair construction')
    mesh.from_pydata([points[i] for i in (18,17,20,19)],[],[(0,1,2),(0,2,3)])
    uv=mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    for loop in mesh.loops:
        weights=inverse@_project(mesh.vertices[loop.vertex_index].co)
        uv.data[loop.index].uv=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
    for material in source.data.materials:mesh.materials.append(material)
    proxy=bpy.data.objects.new(source.name+' / access flight',mesh)
    proxy.parent=source.parent;proxy.matrix_world=Matrix.Identity(4)
    for key in source.keys():proxy[key]=source[key]
    result=add_steps(proxy,(0,1),18)
    obj=bpy.data.objects[result['object']]
    obj['lower_west_stair_refinement']=TAG
    obj['todo']='Concealed lower riser spacing inferred from the source-visible upper flight.'
    obj.data.uv_layers[0].name=source.data.uv_layers[0].name
    obj.data.attributes.new('reprojection_fallback_material','INT','FACE')
    bpy.data.objects.remove(proxy,do_unlink=True)
    bpy.data.meshes.remove(mesh)
    return result


def _round_shaft(source):
    """Replace the small turret's eight planar sides with its round silhouette."""
    points=[source.matrix_world@v.co for v in source.data.vertices]
    low=min(p.z for p in points);high=max(p.z for p in points)
    cx=(min(p.x for p in points)+max(p.x for p in points))/2
    cy=(min(p.y for p in points)+max(p.y for p in points))/2
    # Its plan is elliptical in the measured geometry; retain both radii.
    rx=(max(p.x for p in points)-min(p.x for p in points))/2
    ry=(max(p.y for p in points)-min(p.y for p in points))/2
    vertices=[(cx+rx*math.cos(i*math.tau/48),cy+ry*math.sin(i*math.tau/48),z)
              for z in (low,high) for i in range(48)]
    faces=[tuple(range(47,-1,-1)),tuple(range(48,96))]
    faces.extend((i,(i+1)%48,(i+1)%48+48,i+48) for i in range(48))
    mesh=bpy.data.meshes.new(source.name+' / round masonry')
    mesh.from_pydata(vertices,[],faces)
    mesh.update()
    uv=mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    matrix=source.matrix_world.to_3x3().inverted().transposed()
    donors=[]
    for face in source.data.polygons:
        if len(face.vertices)!=3:continue
        projection=Matrix([_project(points[i]) for i in face.vertices]).transposed()
        if abs(projection.determinant())<1e-7:continue
        donors.append(((matrix@face.normal).normalized(),projection.inverted(),
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices]))
    for face in mesh.polygons:
        normal,inverse,coords=max(donors,key=lambda d:d[0].dot(face.normal))
        for li in face.loop_indices:
            weights=inverse@_project(mesh.vertices[mesh.loops[li].vertex_index].co)
            uv.data[li].uv=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
    for mat in source.data.materials:mesh.materials.append(mat)
    mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    obj=bpy.data.objects.new(source.name+' / round masonry',mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():obj[key]=source[key]
    obj['lower_west_shaft_refinement']=TAG
    source.hide_render=True;source.hide_set(True)
    source['lower_west_shaft_baseline']=TAG
    bm=bmesh.new();bm.from_mesh(mesh)
    bad=sum(not e.is_manifold for e in bm.edges);bm.free()
    if bad:raise RuntimeError('Open turret shaft')
    return {'source_node':obj['source_node'],'radial_segments':48,'nonmanifold_edges':bad}


def _roof(source):
    """Give the five visible roof sectors one shared apex and curved eaves."""
    points = [source.matrix_world @ v.co for v in source.data.vertices]
    center = Vector((371.2, -3894, 0))
    # Adjacent sectors share exact boundaries, avoiding the independently
    # rounded endpoints and multiple apexes of the generated wedges.
    boundaries = {'building-028':(-112,-66), 'building-029':(-158,-112),
                  'building-030':(158,202), 'building-031':(-27,26),
                  'building-032':(-66,-27)}
    a,b = [math.radians(v) for v in boundaries[source['source_node']]]
    segments = max(8,round((b-a)*16))
    ring = [Vector((center.x+36.5*math.cos(a+(b-a)*i/segments),
                    center.y+36.5*math.sin(a+(b-a)*i/segments),249))
            for i in range(segments+1)]
    vertices = [Vector((center.x,center.y,330.5)),Vector((center.x,center.y,245))]
    vertices += ring + [Vector((p.x,p.y,245)) for p in ring]
    n = len(ring)
    faces=[]
    for i in range(segments):
        faces.extend([(0,2+i,3+i),(1,3+n+i,2+n+i),
                      (2+i,2+n+i,3+n+i,3+i)])
    faces.extend([(0,1,2+n,2),(0,1,1+2*n,1+n)])
    mesh=bpy.data.meshes.new(source.name+' / continuous cone sector')
    mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bad=sum(not e.is_manifold for e in bm.edges)
    if bad: raise RuntimeError('Open roof sector')
    bm.to_mesh(mesh);bm.free()
    uv=mesh.uv_layers.new(name=source.data.uv_layers[0].name)
    normal_matrix=source.matrix_world.to_3x3().inverted().transposed()
    donors=[]
    for face in source.data.polygons:
        if len(face.vertices)!=3: continue
        matrix=Matrix([_project(points[i]) for i in face.vertices]).transposed()
        if abs(matrix.determinant())<1e-7: continue
        donors.append(((normal_matrix@face.normal).normalized(),matrix.inverted(),
                       [source.data.uv_layers[0].data[i].uv.copy() for i in face.loop_indices]))
    for face in mesh.polygons:
        normal,inverse,coords=max(donors,key=lambda d:d[0].dot(face.normal))
        for li in face.loop_indices:
            weights=inverse@_project(mesh.vertices[mesh.loops[li].vertex_index].co)
            uv.data[li].uv=sum((coords[k]*weights[k] for k in range(3)),Vector((0,0)))
    for mat in source.data.materials: mesh.materials.append(mat)
    backup=mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    for face in mesh.polygons: backup.data[face.index].value=0
    obj=bpy.data.objects.new(source.name+' / continuous roof sector',mesh)
    bpy.data.collections['Derby Working'].objects.link(obj)
    obj.parent=source.parent;obj.matrix_world=Matrix.Identity(4)
    for key in source.keys(): obj[key]=source[key]
    obj['lower_west_roof_refinement']=TAG
    source.hide_render=True;source.hide_set(True)
    source['lower_west_roof_baseline']=TAG
    return {'source_node':obj['source_node'],'roof_segments':segments,'nonmanifold_edges':bad}


def refine():
    """Replace three connected runs; repeated calls reuse the generated meshes."""
    bpy.context.view_layer.update()
    working = bpy.data.collections['Derby Working']
    existing = [o for o in working.objects if o.get('lower_west_curtain_refinement') == TAG]
    if existing:
        roofs = [o for o in working.objects if o.get('lower_west_roof_refinement') == TAG]
        shafts=[o for o in working.objects if o.get('lower_west_shaft_refinement')==TAG]
        stairs=[o for o in working.objects if o.get('lower_west_stair_refinement')==TAG]
        if len(existing) != 4 or len(roofs) != 5 or len(shafts)!=1 or len(stairs)!=1:
            raise RuntimeError('Incomplete lower west curtain pass')
        return {'reused': True, 'objects': [o.name for o in existing+roofs+shafts+stairs]}
    recipes = {
        'building-023': [
            (76,66,1,((1830,1842),(1858,1870),(1886,1898))),
            (66,67,1,((1938,1950),(1966,1978),(1994,2006),(2022,2034))),
            (80,76,0,((295,310),)),
            (79,80,0,((265,274),)),
            (75,74,0,((267,280),)),
            (74,73,0,((305,319),)),
        ],
        'building-025': [
            (28,27,0,((339,346),(359,366),(379,386))),
            (27,26,0,((339,346),(359,366),(379,386))),
        ],
        'building-042': [
            (35,36,0,((405,414),(426,435),(447,456),(468,477),(489,498))),
        ],
        'building-036': [
            (49,40,0,((270,278),)),
            (40,41,0,((299,312),)),
            (48,47,0,((272,283),)),
        ],
    }
    results = []
    for node, cuts in recipes.items():
        candidates = [o for o in working.objects if o.type == 'MESH'
                      and o.get('source_node') == node and not o.hide_render]
        if len(candidates) != 1:
            raise RuntimeError('Expected one visible baseline for ' + node)
        results.append(_wall(candidates[0], cuts))
    roofs=[]
    for number in range(28,33):
        node=f'building-{number:03d}'
        source=next(o for o in working.objects if o.get('source_node')==node and not o.hide_render)
        roofs.append(_roof(source))
    shaft=_round_shaft(next(o for o in working.objects if o.get('source_node')=='building-027' and not o.hide_render))
    stair=_stair(next(o for o in working.objects if o.get('source_node')=='building-038' and not o.hide_render))
    return {'objects': results, 'notches': sum(r['notches'] for r in results),
            'roofs':roofs,
            'shaft':shaft,
            'stair':stair,
            'remaining': 'Arrow slits and corbels remain to be modeled; concealed roof continuation is uncertain.'}
