"""Southwest cottage reconstruction helpers; owned parts 057 and 058 only.

Roof and wall dimensions are world-space measurements. Profiles retain straight
ridge/eave axes; the roof shoulder is a continuous broad curve, not a silhouette
extrusion. Dimensions are measured from the isolated worker's structural axes.
"""
import math
import bpy
import bmesh
from mathutils import Vector, Matrix
from mathutils.bvhtree import BVHTree

ASSET = 'derby-lower-southwest-cottage'
PARTS = ('building-057', 'building-058')


def closed_roof_half(ridge, eave, *, thickness=4.0, shoulder=5.0, bands=16):
    """Return a closed thatch half-shell with consistent vertical thickness."""
    if thickness <= 0 or bands < 2:
        raise ValueError('Positive thickness and at least two bands required')
    ridge = [Vector(p) for p in ridge]
    eave = [Vector(p) for p in eave]
    if len(ridge) != 2 or len(eave) != 2:
        raise ValueError('Roof axes must each have exactly two endpoints')
    vertices = []
    for i in range(bands + 1):
        t = i / bands
        for start, end in zip(ridge, eave):
            p = start.lerp(end, t)
            p.z += shoulder * 4 * t * (1 - t)
            vertices.append(p)
    count = len(vertices)
    vertices += [p - Vector((0, 0, thickness)) for p in vertices]
    faces = []
    for i in range(bands):
        a = 2 * i
        faces += [(a, a + 1, a + 3, a + 2),
                  (count + a + 2, count + a + 3, count + a + 1, count + a)]
    perimeter = [0, 1] + list(range(3, count, 2)) + list(range(count - 2, 0, -2))
    for a, b in zip(perimeter, perimeter[1:] + perimeter[:1]):
        faces.append((a, b, count + b, count + a))
    return vertices, faces


def beam_between(start, end, *, width, depth):
    """Closed rectangular timber aligned with its measured end joints."""
    start, end = Vector(start), Vector(end)
    axis = end - start
    if axis.length < 1e-6 or min(width, depth) <= 0:
        raise ValueError('Timber requires distinct endpoints and positive section')
    direction = axis.normalized()
    transverse = direction.cross(Vector((0, 0, 1)))
    if transverse.length < 1e-6:
        transverse = direction.cross(Vector((1, 0, 0)))
    transverse.normalize()
    vertical = transverse.cross(direction).normalized()
    vertices = [p + transverse * (sx * width / 2) + vertical * (sy * depth / 2)
                for p in (start, end)
                for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    faces = [(3, 2, 1, 0), (4, 5, 6, 7),
             (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    return vertices, faces


def validate_axes(ridge, eave, *, tolerance=0.05):
    """Reject accidentally warped axes before creating architectural geometry."""
    for name, points in [('ridge', ridge), ('eave', eave)]:
        if abs(points[0][2] - points[1][2]) > tolerance:
            raise ValueError(f'{name} must remain level; inspect measured endpoints')
    ridge_direction = Vector(ridge[1]) - Vector(ridge[0])
    eave_direction = Vector(eave[1]) - Vector(eave[0])
    if min(ridge_direction.length, eave_direction.length) < 1e-6:
        raise ValueError('Roof axes must have nonzero length')
    if abs(ridge_direction.normalized().dot(eave_direction.normalized())) < .999:
        raise ValueError('Ridge and eave axes must remain parallel')


def _mesh(name, vertices, faces):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh); bm.free()
    mesh.uv_layers.new(name='Source projection placeholder')
    return mesh


def _wall(ridge, eave):
    axis = (ridge[1] - ridge[0]).normalized()
    vertices = []
    for i in range(17):
        t = .90 * i / 16
        for j in range(2):
            p = ridge[j].lerp(eave[j], t)
            p.z += 20 * t * (1-t) - 4.0
            p += axis * (3 if j == 0 else -3)
            vertices.append(p)
    count = len(vertices)
    vertices += [Vector((p.x, p.y, 0)) for p in vertices]
    faces = []
    for i in range(16):
        a = 2*i
        faces += [(a,a+1,a+3,a+2), (count+a+2,count+a+3,count+a+1,count+a)]
    perimeter = [0,1] + list(range(3,count,2)) + list(range(count-2,0,-2))
    faces += [(a,b,count+b,count+a) for a,b in zip(perimeter,perimeter[1:]+perimeter[:1])]
    return vertices, faces


def _join_geometry(obj, geometry):
    verts = [v.co.copy() for v in obj.data.vertices]
    faces = [tuple(p.vertices) for p in obj.data.polygons]
    for extra, polygons in geometry:
        offset = len(verts)
        verts.extend(extra)
        faces.extend(tuple(offset+i for i in face) for face in polygons)
    obj.data = _mesh(obj.name+' / round two', verts, faces)


def _cut_opening(obj, outline):
    count = len(outline)
    vertices = [p + Vector((0,0,dz)) for dz in (2,-17) for p in outline]
    faces = [tuple(range(count)),tuple(range(count,2*count))]
    faces += [(i,(i+1)%count,(i+1)%count+count,i+count) for i in range(count)]
    mesh = _mesh('Temporary damaged thatch cutter',vertices,faces)
    cutter = bpy.data.objects.new(mesh.name,mesh)
    bpy.context.scene.collection.objects.link(cutter)
    modifier = obj.modifiers.new('Damaged thatch pocket','BOOLEAN')
    modifier.operation='DIFFERENCE';modifier.solver='EXACT';modifier.object=cutter
    bpy.context.view_layer.objects.active=obj
    try:
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    finally:
        bpy.data.objects.remove(cutter,do_unlink=True)
        if mesh.users == 0: bpy.data.meshes.remove(mesh)


def refine():
    collection = bpy.data.collections['Derby Working']
    owned = [o for o in collection.objects if o.type == 'MESH'
             and o.get('asset_group') == ASSET and not o.hide_render]
    if len(owned) != 2 or {o.get('source_node') for o in owned} != set(PARTS):
        raise ValueError('Expected exactly the two prepared cottage half-shells')
    if any(o.get('southwest_round2') for o in owned):
        return {'status': 'existing'}
    west = next(o for o in owned if o['source_node'] == PARTS[0])
    world = [west.matrix_world @ v.co for v in west.data.vertices]
    ridge = [world[0].copy(),world[1].copy()]
    level = sum(p.z for p in ridge)/2
    for p in ridge: p.z = level
    axis = ridge[1]-ridge[0]
    neutral = bpy.data.materials.new('Southwest cottage round two source pending')
    neutral.diffuse_color = (.27,.27,.27,1)
    report = []
    for obj in owned:
        oldworld = [obj.matrix_world @ v.co for v in obj.data.vertices]
        ids = (8,9) if obj == west else (10,13)
        eave = [oldworld[i].copy() for i in ids]
        z = sum(p.z for p in eave)/2
        eave[0].z = z
        eave[1] = eave[0] + axis
        validate_axes(ridge,eave)
        obj.data = _mesh(obj.name+' / thick thatch', *closed_roof_half(ridge,eave))
        obj.matrix_world = Matrix.Identity(4)
        extras = [_wall(ridge,eave)]
        timbers = []
        if obj != west:
            # Locate the observed diagonal timber on the uncut roof surface.
            bm = bmesh.new(); bm.from_mesh(obj.data)
            tree = BVHTree.FromBMesh(bm)
            sin,cos = math.sin(math.radians(35)), math.cos(math.radians(35))
            forward = Vector((0,cos,-sin))
            points = []
            for x,y in [(529,2168),(550,2153)]:
                origin = Vector((x,-y*sin,-y*cos))-forward*10000
                hit,normal,face,distance = tree.ray_cast(origin,forward)
                if hit is None: raise ValueError('Observed rafter endpoint misses roof')
                points.append(hit-Vector((0,0,2.0)))
            outline=[]
            for x,y in [(532,2152),(548,2153),(552,2161),(546,2169),(536,2171),(529,2165)]:
                origin=Vector((x,-y*sin,-y*cos))-forward*10000
                hit,normal,face,distance=tree.ray_cast(origin,forward)
                if hit is None: raise ValueError('Damaged roof outline misses roof')
                outline.append(hit)
            bm.free()
            timbers.append(beam_between(*points,width=3.2,depth=3.0))
            _cut_opening(obj,outline)
            # Apply the pocket separately to the supporting volume. Applying
            # an exact boolean to overlapping disconnected shells is ambiguous.
            wall_mesh = _mesh('Temporary cottage supporting volume', *extras[0])
            wall = bpy.data.objects.new(wall_mesh.name,wall_mesh)
            collection.objects.link(wall)
            try:
                _cut_opening(wall,outline)
                extras[0] = ([v.co.copy() for v in wall.data.vertices],
                             [tuple(p.vertices) for p in wall.data.polygons])
            finally:
                bpy.data.objects.remove(wall,do_unlink=True)
                if wall_mesh.users == 0: bpy.data.meshes.remove(wall_mesh)
        _join_geometry(obj,extras)
        if obj != west:
            _join_geometry(obj,timbers)
        obj.data.materials.append(neutral)
        obj['projection_min_cosine'] = .2
        obj['southwest_round2'] = 'thick-thatch-supported-eaves-and-rafter'
        bm = bmesh.new(); bm.from_mesh(obj.data)
        defects = {'nonmanifold':sum(not e.is_manifold for e in bm.edges),
                   'degenerate':sum(f.calc_area()<1e-7 for f in bm.faces)}
        bm.free()
        if any(defects.values()): raise ValueError(str(defects))
        report.append({'part':obj['source_node'],'vertices':len(obj.data.vertices),
                       'faces':len(obj.data.polygons),'validation':defects,
                       'ridge_z':level,'eave_z':z,'roof_thickness':4,
                       'wall_inset_fraction':.1,'gable_inset':3})
    bpy.context.view_layer.update()
    return {'status':'refined','objects':report}
