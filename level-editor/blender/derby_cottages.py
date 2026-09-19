"""Source-aligned cottage details; run refine() in the Derby working scene.

The west cottage porch has a painted round-headed recessed doorway. Keep the
existing roof and outer outline; replace the flat front with a shallow recess.
Dimensions are estimated from the reference artwork, not collision metadata.
"""
import math
import bpy
import bmesh
from mathutils import Vector, Matrix


def refine():
    working = bpy.data.collections['Derby Working']
    existing = [o for o in working.all_objects if o.get('cottage_refinement')]
    if existing:
        if len(existing) != 1 or existing[0].get('source_node') != 'building-053':
            raise ValueError('Conflicting cottage refinement')
        return {'object': existing[0].name, 'status': 'existing'}
    source = next(o for o in working.all_objects
                  if o.get('source_node') == 'building-053' and not o.hide_render)
    bpy.context.view_layer.update()
    world = [source.matrix_world @ v.co for v in source.data.vertices]
    # Use the roof's four corners to close the independently offset source faces.
    left, right, back_left, back_right = (world[i] for i in (18, 19, 17, 16))
    elevation = math.radians(35)
    def screen(v):
        return Vector((v.x, -v.y * math.sin(elevation) - v.z * math.cos(elevation), 1))
    projections = {}
    for face_id in (0, 2, 4, 6, 8):
        face = source.data.polygons[face_id]
        matrix = Matrix([screen(world[i]) for i in face.vertices]).transposed().inverted()
        uv = [source.data.uv_layers.active.data[i].uv.copy() for i in face.loop_indices]
        projections[face_id] = (matrix, uv)
    verts, faces, face_sources = [], [], []
    def point(u, depth, z):
        front = left.lerp(right, u)
        back = back_left.lerp(back_right, u)
        v = front.lerp(back, depth)
        v.z = z
        return v
    def add(points, mapping):
        indices = []
        for p in points:
            indices.append(len(verts)); verts.append(p)
        faces.append(indices); face_sources.append(mapping)
    low, high, spring, depth = .33, .65, 21.5, .24
    radius = (right-left).length * (high-low) / 2
    arch = [(low, 1.5), (low, spring)]
    for i in range(1, 17):
        theta = math.pi * (1-i/16)
        arch.append(((low+high)/2 + (high-low)/2*math.cos(theta), spring+radius*math.sin(theta)))
    arch.append((high, 1.5))
    front_arch = [point(u, 0, z) for u,z in arch]
    back_arch = [point(u, depth, z) for u,z in arch]
    fl, fr = point(0,0,0), point(1,0,0)
    bl, br = point(0,1,0), point(1,1,0)
    add([fl]+front_arch+[fr, right, left], 2)
    add(back_arch, 2)
    for i in range(len(arch)-1):
        add([front_arch[i], back_arch[i], back_arch[i+1], front_arch[i+1]], 2)
    # A low sill leaves solid masonry underneath the recessed doorway.
    add([front_arch[0], front_arch[-1], back_arch[-1], back_arch[0]], 2)
    add([fl, fr, front_arch[-1], front_arch[0]], 2)
    add([fl,fr,br,bl], 4)
    add([fl,bl,back_left,left], 4)
    add([fr,right,back_right,br], 0)
    add([bl,br,back_right,back_left], 6)
    add([left,back_left,back_right,right], 8)
    mesh = bpy.data.meshes.new('West cottage porch recessed arch')
    mesh.from_pydata(verts, [], faces); mesh.update()
    uv_layer = mesh.uv_layers.new(name='UVMap')
    for polygon, mapping in zip(mesh.polygons, face_sources):
        inverse, triangle_uv = projections[mapping]
        for loop_id in polygon.loop_indices:
            weights = inverse @ screen(verts[mesh.loops[loop_id].vertex_index])
            uv_layer.data[loop_id].uv = sum((triangle_uv[i]*weights[i] for i in range(3)), Vector((0,0)))
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.0001)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    # Triangulate the concave wall ring explicitly for identical glTF rendering.
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    defects = {'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
               'degenerate_faces':sum(f.calc_area()<1e-6 for f in bm.faces)}
    if any(defects.values()):
        bm.free(); bpy.data.meshes.remove(mesh)
        raise ValueError(f'Invalid porch geometry: {defects}')
    bm.to_mesh(mesh); bm.free()
    for material in source.data.materials: mesh.materials.append(material)
    obj = bpy.data.objects.new(source.name+' / Recessed arch', mesh)
    working.objects.link(obj)
    obj.parent = source.parent; obj.matrix_world = Matrix.Identity(4)
    for key in source.keys(): obj[key] = source[key]
    obj['cottage_refinement'] = 'west-porch-arched-recess-v1'
    obj['part_name'] = 'Porch roof and recessed arched doorway'
    source.hide_render = source.hide_viewport = True
    bpy.context.view_layer.update()
    return {'source':'building-053','object':obj.name,'faces':len(mesh.polygons),
            'recess_depth_fraction':depth,'validation':defects}
