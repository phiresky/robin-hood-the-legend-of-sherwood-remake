"""Measured Great Keep rooftop parapet correction.

The narrow rear crenellated strip belongs to the north tower terrace. Its
collision extrusion previously continued 835 units below its supporting roof,
making an isolated vertical blade visible from the side and rear.
"""
import bpy
import bmesh
import math
from mathutils import Vector, Matrix


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


def refine_fireplace():
    """Build the painted hall fireplace as a shallow hood and open hearth.

    Four measured hood corners determine front width and slope. The existing
    rear room wall and floor anchor the volume; hidden depth is conservative.
    This component belongs to the revealed hall receiver, never its cover.
    """
    working = bpy.data.collections['Derby Working']
    tag = 'great-keep-hall-fireplace-v1'
    if any(o.get('round2_keep_recipe') == tag for o in working.objects):
        return {'status': 'already-applied'}
    source = next(o for o in working.objects if o.type == 'MESH'
                  and not o.hide_render and o.get('source_node') == 'building-227')
    normal = Vector((-.544638991, -.83867067, 0))
    tangent = Vector((.83867067, -.544638991, 0))
    wall = Vector((970, -1644.329, 0))
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def source_point(x, y, depth):
        world_y = wall.y + (depth - normal.x * (x-wall.x)) / normal.y
        return Vector((x, world_y, (-y-world_y*sine)/cosine))

    top = [source_point(946, 661, 3), source_point(995, 679, 3)]
    bottom = [source_point(932, 719, 28), source_point(981, 736, 28)]
    for ring in (top, bottom):
        level = sum(v.z for v in ring)/2
        for point in ring: point.z = level
    vertices, faces = [], []

    def prism(front, depth):
        base = len(vertices)
        vertices.extend(front)
        vertices.extend(p-normal*depth for p in front)
        faces.extend(tuple(base+i for i in f) for f in
                     ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),
                      (2,6,7,3),(3,7,4,0)))

    # Sloping hood: rear points land on the existing masonry wall.
    front = [bottom[0], bottom[1], top[1], top[0]]
    base = len(vertices)
    vertices.extend(front)
    vertices.extend([bottom[0]-normal*28, bottom[1]-normal*28,
                     top[1]-normal*3, top[0]-normal*3])
    faces.extend(tuple(base+i for i in f) for f in
                 ((0,1,2,3),(7,6,5,4),(0,4,5,1),(1,5,6,2),
                  (2,6,7,3),(3,7,4,0)))
    z_bottom = bottom[0].z
    left, right = bottom[0]-tangent*1.5+normal*1.5, bottom[1]+tangent*1.5+normal*1.5
    prism([left-Vector((0,0,5)), right-Vector((0,0,5)), right, left], 30)
    # Side jambs leave an actual empty hearth opening backed by the room wall.
    for a, b in ((bottom[0],bottom[0]+tangent*4),
                 (bottom[1]-tangent*4,bottom[1])):
        low_a, low_b = a.copy(), b.copy()
        low_a.z = low_b.z = 219.75
        high_a, high_b = a.copy(), b.copy()
        high_a.z = high_b.z = z_bottom-5
        prism([low_a,low_b,high_b,high_a],28)
    mesh = bpy.data.meshes.new('Great Keep / revealed hall fireplace mesh')
    mesh.from_pydata(vertices, [], faces)
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges=sum(not e.is_manifold for e in bm.edges)
    bad_faces=sum(f.calc_area()<1e-8 for f in bm.faces)
    bm.to_mesh(mesh); bm.free()
    if bad_edges or bad_faces: raise ValueError('Invalid fireplace component')
    neutral=bpy.data.materials.get('Great Keep / fireplace unknown')
    if neutral is None:
        neutral=bpy.data.materials.new('Great Keep / fireplace unknown')
        neutral.diffuse_color=(.25,.25,.25,1)
    mesh.materials.append(neutral)
    for original in source.data.uv_layers:
        layer=mesh.uv_layers.new(name=original.name)
        for uv in layer.data: uv.uv=(.5,.5)
    obj=bpy.data.objects.new('Great Keep / Hall revealed fireplace',mesh)
    working.objects.link(obj); obj.parent=source.parent; obj.matrix_world=Matrix.Identity(4)
    for key in source.keys():
        if not key.startswith('reprojection_'): obj[key]=source[key]
    obj['round2_keep_recipe']=tag
    obj['round2_keep_component']='hall-fireplace'
    return {'source_node':'building-227','new_mesh':obj.name,'faces':len(faces),
            'nonmanifold_edges':bad_edges,'degenerate_faces':bad_faces,
            'floor':219.75,'mantel':z_bottom,'hood_top':top[0].z,
            'inferred_depth':28,'source_state':'patch-000 revealed',
            'source_hood_corners':[[946,661],[995,679],[981,736],[932,719]]}
