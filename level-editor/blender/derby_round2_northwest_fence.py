"""Reconstruct the northwest yard fence from reviewed exterior timber runs.

Masks 26 and 25 confirm the rear assembly and near cross-fence respectively.
Their silhouettes constrain projection, not hidden timber depth. The
screen-space footing path below remains an explicit reconstruction hypothesis.
"""
import math

import bmesh
import bpy
from mathutils import Vector

ASSET = 'derby-lower-northwest-yard-fence'
NODE = 'building-063'
TAG = 'derby-round2-northwest-fence-v4'
SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))

# Screen-space ground contacts. The western runs follow the retained footprint;
# the eastern returns follow the visible yard boundary and mask foot line.
FOOT_PATH = ((445.9, 1792.0), (482.6, 1746.4), (600.0, 1717.0),
             (647.0, 1718.0), (663.0, 1767.0), (660.0, 1792.0))
POST_FRACTIONS = ((.13, .42, .76, 1), (0, .19, .41, .64, .84, 1),
                  (0, .48, 1), (0, .5, 1), (0, 1))


def refine():
    candidates = [o for o in bpy.data.collections['Derby Working'].objects
                  if o.type == 'MESH' and not o.hide_render
                  and o.get('source_node') == NODE and o.get('asset_group') == ASSET
                  and o.get('projection_component') != 'near-cross-fence']
    if len(candidates) != 1:
        raise ValueError(f'Expected one visible fence mesh, got {len(candidates)}')
    source = candidates[0]
    points, faces = [], []

    def timber(a, b, width, height, side_hint=None):
        """Closed straight rectangular member, with no silhouette-pixel zigzags."""
        direction = (b - a).normalized()
        side = side_hint.copy() if side_hint is not None else direction.cross(Vector((0, 0, 1)))
        if side.length < 1e-8:
            side = Vector((1, 0, 0))
        side.normalize()
        up = side.cross(direction).normalized()
        side *= width / 2
        up *= height / 2
        ring = (-side-up, side-up, side+up, -side+up)
        start = len(points)
        points.extend([p + offset for p in (a, b) for offset in ring])
        faces.extend([tuple(start+i for i in face) for face in
                      ((3, 2, 1, 0), (4, 5, 6, 7), (0, 1, 5, 4),
                       (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7))])

    path = [Vector((x, -y / SIN, 0)) for x, y in FOOT_PATH]
    posts = []
    for segment, (a, b) in enumerate(zip(path, path[1:])):
        for z in ((8.75, 21.75) if segment < 2 else (8., 18.5, 29.5)):
            timber(a + Vector((0, 0, z)), b + Vector((0, 0, z)), 2.1, 3.5)
        for fraction in POST_FRACTIONS[segment]:
            center = a.lerp(b, fraction)
            if any((center - previous).length < .01 for previous in posts):
                continue
            posts.append(center)
            timber(center, center + Vector((0, 0, 33.5)), 3.0, 3.0)

    # Broad upright palings are visible in the source and in mask 26. Sparse
    # posts alone miss most of the western fence silhouette. These are straight
    # boards with regular planar caps, not extruded raster contours.
    palings = ((450., 457.5, 465., 472.5, 479.),
               (483.5, 491.5, 504., 515.5, 526., 535.5,
                548., 557., 566., 575., 584.5, 595.5))
    for segment, centers in enumerate(palings):
        a, b = path[segment:segment+2]
        tangent = (b-a).normalized()
        for x in centers:
            center = a.lerp(b, (x-a.x)/(b.x-a.x))
            width = (5.5 if segment == 0 else 7.1) / abs(tangent.x)
            timber(center, center+Vector((0, 0, 33.5)), width, 1.8,
                   side_hint=tangent)

    inverse = source.matrix_world.inverted()
    mesh = bpy.data.meshes.new('Northwest yard fence / complete open rail assembly')
    mesh.from_pydata([inverse @ p for p in points], [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-8 for f in bm.faces)
    if bad_edges or bad_faces:
        bm.free()
        bpy.data.meshes.remove(mesh)
        raise ValueError(f'Fence has {bad_edges} nonmanifold edges and {bad_faces} degenerate faces')
    bm.to_mesh(mesh)
    bm.free()

    # This provisional UV layer is replaced by the workspace source-ownership bake.
    for material in source.data.materials:
        mesh.materials.append(material)
    uv = mesh.uv_layers.new(name='Fence source projection')
    uv.active_render = True
    for loop in mesh.loops:
        p = source.matrix_world @ mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv = (p.x / 1920, 1 - (-p.y*SIN-p.z*COS) / 2752)
    fallback = mesh.attributes.new('reprojection_fallback_material', 'INT', 'FACE')
    for face in fallback.data:
        face.value = 0
    source.data = mesh
    source['refinement_recipe'] = TAG
    source['reviewed_occlusion_masks'] = '[26]'
    # Edge-on east timbers occupy only a few source pixels. Reject those side
    # faces instead of stretching alternating rail/soil pixels along a full run.
    source['projection_min_cosine'] = .2
    source['todo'] = 'Verify ground contacts, rail alignment and eastern return silhouette against mask 26; hidden timber depth remains inferred.'
    source.name = 'Lower Bailey Northwest Yard Fence / Connected timber rails and posts'
    near_report = refine_near_return()
    return {'asset': ASSET, 'source_node': NODE, 'posts': len(posts),
            'near_return': near_report,
            'palings': sum(map(len, palings)),
            'runs': len(path)-1, 'faces': len(mesh.polygons),
            'nonmanifold_edges': bad_edges, 'degenerate_faces': bad_faces,
            'status': 'Geometry generated; source reprojection and visual review required'}


def refine_near_return():
    """Add the five-rail near return separately so its source ownership stays local."""
    visible = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == NODE and o.get('asset_group') == ASSET]
    rear = next(o for o in visible if o.get('projection_component') != 'near-cross-fence')
    existing = [o for o in visible if o.get('projection_component') == 'near-cross-fence']
    if len(existing) > 1:
        raise ValueError('Duplicate near cross-fence components')
    points, faces = [], []

    def beam(a, b, width=2.1, height=2.6):
        tangent = (b-a).normalized()
        side = tangent.cross(Vector((0, 0, 1)))
        if side.length < 1e-8:
            side = Vector((1, 0, 0))
        side.normalize()
        up = side.cross(tangent).normalized()
        offsets = [-side*width/2-up*height/2, side*width/2-up*height/2,
                   side*width/2+up*height/2, -side*width/2+up*height/2]
        start = len(points)
        points.extend(p+offset for p in (a,b) for offset in offsets)
        faces.extend(tuple(start+i for i in face) for face in
                     ((3,2,1,0),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)))

    west = Vector((594, -1807/SIN, 0))
    east = Vector((661, -1798/SIN, 0))
    for height_w, height_e in zip((1.,7.,12.,19.5,26.5), (1.,7.7,16.,24.5,31.5)):
        beam(west+Vector((0,0,height_w)),east+Vector((0,0,height_e)))
    for x, height in ((594,34.),(620,35.),(661,47.)):
        foot=west.lerp(east,(x-west.x)/(east.x-west.x))
        beam(foot,foot+Vector((0,0,height)),width=3.,height=3.)
    rear_end=Vector((660,-1792/SIN,0))
    for z in (8.,18.5,29.5):
        beam(rear_end+Vector((0,0,z)),east+Vector((0,0,z)),height=3.5)

    mesh=bpy.data.meshes.new('Northwest yard / near cross-fence')
    inverse=rear.matrix_world.inverted()
    mesh.from_pydata([inverse@p for p in points],[],faces)
    mesh.update()
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    bad_edges=sum(not edge.is_manifold for edge in bm.edges)
    bad_faces=sum(face.calc_area()<1e-8 for face in bm.faces)
    bm.to_mesh(mesh);bm.free()
    if bad_edges or bad_faces:
        raise ValueError(f'Near fence invalid: {bad_edges} edges, {bad_faces} faces')
    for material in rear.data.materials:
        mesh.materials.append(material)
    uv=mesh.uv_layers.new(name='Near fence provisional source projection')
    uv.active_render=True
    for loop in mesh.loops:
        p=rear.matrix_world@mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(p.x/1920,1-(-p.y*SIN-p.z*COS)/2752)
    mesh.attributes.new('reprojection_fallback_material','INT','FACE')
    obj=existing[0] if existing else bpy.data.objects.new('Lower Bailey Northwest Yard Fence / Near cross-fence',mesh)
    if existing:
        obj.data=mesh
    else:
        bpy.data.collections['Derby Working'].objects.link(obj)
        obj.parent=rear.parent
        obj.matrix_world=rear.matrix_world.copy()
        for key in rear.keys():
            obj[key]=rear[key]
    obj['projection_component']='near-cross-fence'
    obj['refinement_recipe']='derby-round2-northwest-fence-near-return-v2'
    obj['reviewed_occlusion_masks']='[25]'
    obj['part_name']='Near yard cross-fence'
    obj['todo']='Hidden timber depths remain inferred; reviewed source evidence is mask25 minus foreground cottage11.'
    rear['projection_component']='rear-yard-fence'
    bpy.context.view_layer.update()
    return {'component':obj['projection_component'],'faces':len(mesh.polygons),
            'rails':5,'posts':3,'joint_connectors':3,
            'nonmanifold_edges':bad_edges,'degenerate_faces':bad_faces}
