"""Recover the short masonry approach stair from the bridge's ramp envelope.

The three risers are a source-art interpretation, not navigation metadata.
Run only in the isolated keep-to-hall bridge refinement workspace.
"""
import bpy
import bmesh
import math
from mathutils import Vector

ASSET = 'derby-keep-hall-bridge'
TAG = 'keep-hall-bridge-round2-steps-v1'


def repair_parapet(collection):
    objects = [o for o in collection.all_objects if o.type == 'MESH'
               and o.get('source_node') == 'building-126']
    obj = next(o for o in objects if not o.hide_render)
    if obj.get('round2_bridge_join') == TAG:
        return {'status': 'existing'}
    source = next(o for o in objects if o.hide_render
                  and not o.get('bridge_arch_refinement'))
    ring = []
    for i in range(24, 30):
        p = source.matrix_world @ source.data.vertices[i].co
        ring.append(Vector((p.x, p.z, -p.y)))
    # The collision return doubles back into a long acute spike. Removing its
    # isolated extra corner leaves the visible outside and straight inner
    # parapet edges unchanged, connecting the actual masonry return directly.
    top = [p for i, p in enumerate(ring) if i != 2]
    count = len(top)
    vertices = top + [Vector((p.x, p.y, 0)) for p in top]
    faces = [tuple(range(count)), tuple(reversed(range(count, 2*count)))]
    faces += [(i, (i+1)%count, (i+1)%count+count, i+count)
              for i in range(count)]
    mesh = bpy.data.meshes.new('Bridge parapet with coherent landing return')
    inverse = obj.matrix_world.inverted()
    mesh.from_pydata([inverse @ p for p in vertices], [], faces)
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(mesh); bm.free()
    for material in obj.data.materials:
        mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data = mesh
    start, end = ring[0].copy(), ring[-1].copy()
    start.z = end.z = 0
    axis = end-start
    normal = Vector((-axis.y, axis.x, 0)).normalized()
    profile = [(.07, -20), (.86, -20), (.86, 12)]
    profile += [(.465+.395*math.cos(math.pi*i/32),
                 12+91*math.sin(math.pi*i/32)) for i in range(1,33)]
    vv = [start+axis*u+normal*d+Vector((0,0,z))
          for d in (-100,100) for u,z in profile]
    n = len(profile)
    ff = [tuple(reversed(range(n))), tuple(range(n,n*2))]
    ff += [(i,(i+1)%n,(i+1)%n+n,i+n) for i in range(n)]
    cm = bpy.data.meshes.new('Temporary existing arch profile')
    cm.from_pydata(vv, [], ff)
    bm=bmesh.new(); bm.from_mesh(cm)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(cm); bm.free()
    cutter = bpy.data.objects.new('Temporary existing arch profile', cm)
    collection.objects.link(cutter)
    bpy.context.view_layer.update()
    try:
        modifier = obj.modifiers.new('Retain reviewed open arch', 'BOOLEAN')
        modifier.operation = 'DIFFERENCE'; modifier.solver = 'EXACT'
        modifier.object = cutter
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=modifier.name)
    finally:
        bpy.data.objects.remove(cutter, do_unlink=True)
        bpy.data.meshes.remove(cm)
    bm=bmesh.new(); bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    defects = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
               'degenerate_faces': sum(f.calc_area() < 1e-7 for f in bm.faces)}
    bm.to_mesh(obj.data); bm.free()
    if any(defects.values()):
        raise ValueError(defects)
    obj['round2_bridge_join'] = TAG
    return {'status': 'refined', 'removed_spike_corner': 26,
            'preserved_arch_crown': 103, 'validation': defects}


def refine():
    collection = bpy.data.collections['Derby Working']
    parapet = repair_parapet(collection)
    candidates = [o for o in collection.all_objects
                  if o.type == 'MESH' and o.get('source_node') == 'building-122']
    active = [o for o in candidates if not o.hide_render]
    if len(active) != 1:
        raise ValueError('Expected exactly one visible west bridge landing')
    obj = active[0]
    if obj.get('round2_bridge_refinement') == TAG:
        return {'status': 'existing', 'source_node': 'building-122',
                'parapet': parapet}
    # Retired source top corners retain the footprint before triangulation.
    retired = [o for o in candidates if o.hide_render
               and not o.get('bridge_arch_refinement')]
    if len(retired) != 1 or len(retired[0].data.vertices) < 20:
        raise ValueError('Missing unambiguous original landing footprint')
    source = retired[0]
    # Retired collision objects retain their import-axis transform; the visible
    # masonry and review cameras use Z-up coordinates.
    corners = [Vector((p.x, p.z, -p.y)) for p in
               [source.matrix_world @ source.data.vertices[i].co
                for i in range(16, 20)]]
    by_height = sorted(corners, key=lambda p: p.z)
    low, high = by_height[:2], by_height[2:]
    if not 16 < sum(p.z for p in high) / 2 - sum(p.z for p in low) / 2 < 21:
        raise ValueError('Landing rise differs from reviewed ramp envelope')
    # Pair cross-stair endpoints by minimum total horizontal travel.
    if sum((low[i].xy-high[i].xy).length for i in (0, 1)) > sum(
            (low[i].xy-high[1-i].xy).length for i in (0, 1)):
        high.reverse()
    base = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
    # Closed extruded stair section: preserve both endpoint levels, distribute
    # three rises over the existing short run, and provide actual level treads.
    profiles = []
    for side in (0, 1):
        a, b = low[side], high[side]
        profile = [Vector((a.x, a.y, base)), a.copy()]
        for step in range(3):
            start = a.lerp(b, step / 3)
            end = a.lerp(b, (step + 1) / 3)
            height = a.z + (b.z-a.z) * (step + 1) / 3
            profile += [Vector((start.x, start.y, height)),
                        Vector((end.x, end.y, height))]
        profile.append(Vector((b.x, b.y, base)))
        profiles.append(profile)
    count = len(profiles[0])
    verts = profiles[0] + profiles[1]
    faces = [tuple(reversed(range(count))), tuple(range(count, count * 2))]
    faces += [(i, (i+1) % count, (i+1) % count + count, i + count)
              for i in range(count)]
    mesh = bpy.data.meshes.new('West bridge landing — three masonry risers')
    inverse = obj.matrix_world.inverted()
    mesh.from_pydata([inverse @ p for p in verts], [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    defects = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
               'degenerate_faces': sum(f.calc_area() < 1e-7 for f in bm.faces)}
    if any(defects.values()):
        bm.free()
        bpy.data.meshes.remove(mesh)
        raise ValueError(defects)
    bm.to_mesh(mesh)
    bm.free()
    for material in obj.data.materials:
        mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj.data = mesh
    obj['round2_bridge_refinement'] = TAG
    return {'status': 'refined', 'source_node': 'building-122',
            'risers': 3, 'validation': defects, 'parapet': parapet,
            'limitations': ['Arch and parapet joints require separate geometry inspection']}
