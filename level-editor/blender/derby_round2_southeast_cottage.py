"""Refine southeast thatch edges and canopy without changing measured roof rails."""
import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-lower-southeast-cottage'
RECIPE = 'southeast-round2-closed-eaves-canopy-foundation-v1'


def replace(obj, vertices, faces):
    mesh = bpy.data.meshes.new(obj.name + ' round2')
    inv = obj.matrix_world.inverted()
    mesh.from_pydata([inv @ Vector(v) for v in vertices], [], faces)
    mesh.update()
    # Projection is reapplied after the geometric review; do not interpolate an
    # old atlas through changed topology.
    mat = bpy.data.materials.get('Southeast round2 neutral')
    if mat is None:
        mat = bpy.data.materials.new('Southeast round2 neutral')
        mat.diffuse_color = (.3, .3, .3, 1)
    mesh.materials.append(mat)
    mesh.uv_layers.new(name='UVMap')
    mesh.attributes.new('reprojection_fallback_material', 'INT', 'FACE')
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.00001)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bad = {'nonmanifold': sum(not e.is_manifold for e in bm.edges),
           'degenerate': sum(f.calc_area() < 1e-7 for f in bm.faces)}
    if any(bad.values()):
        raise ValueError((obj.name, bad))
    bm.to_mesh(mesh)
    bm.free()
    obj.data = mesh
    obj['refinement_recipe'] = RECIPE
    obj['southeast_round2'] = True
    return {'object': obj.name, 'source_node': obj['source_node'],
            'vertices': len(mesh.vertices), 'faces': len(mesh.polygons), **bad}


def slab(obj, top, thickness):
    bottom = [p - Vector((0, 0, thickness)) for p in top]
    return replace(obj, top + bottom,
                   [(0, 1, 2, 3), (7, 6, 5, 4)] +
                   [(i, (i+1) % 4, (i+1) % 4+4, i+4) for i in range(4)])


def refine():
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render and o.get('asset_group') == ASSET]
    if objects and all(o.get('southeast_round2') for o in objects):
        return {'status': 'existing', 'recipe': RECIPE}
    if len(objects) != 6:
        raise ValueError('Expected six original southeast components')
    world = {o.name: [o.matrix_world @ v.co for v in o.data.vertices] for o in objects}
    report = []
    roofs = [o for o in objects if 'thatch roof' in o.name]
    for obj in roofs:
        w = world[obj.name]
        # Both measured long rails are horizontal; keep their distinct heights.
        for a, b in ((0, 3), (1, 2)):
            w[a].z = w[b].z = (w[a].z + w[b].z) / 2
        vertices = []
        # Broad planar slope, then two narrow rolled-thatch edge bands. The
        # underside stays on the measured supporting wall plane.
        fractions = (0, .82, .95, 1)
        for lower in (False, True):
            for end, ridge in ((0, 1), (3, 2)):
                for t in fractions:
                    p = w[ridge].lerp(w[end], t)
                    p.z += (-3 if lower else (0 if t in (0, 1) else .75))
                    if not lower and t == 1:
                        p.z -= .6
                    vertices.append(p)
        faces = []
        for k in range(3):
            faces += [(k, k+1, k+5, k+4), (k+8, k+12, k+13, k+9)]
        for base in (0, 4):
            for k in range(3):
                faces.append((base+k, base+k+8, base+k+9, base+k+1))
        faces += [(0, 4, 12, 8), (3, 11, 15, 7)]
        report.append(replace(obj, vertices, faces))
    porch = next(o for o in objects if o['source_node'] == 'building-059')
    w = world[porch.name]
    old_faces = [tuple(p.vertices) for p in porch.data.polygons]
    top = [w[i].copy() for i in (4, 5, 6, 7)]
    # Leave the documented door recess in place: the artwork cannot establish
    # a fully open porch. Separate canopy thickness from the supporting shell.
    for i in (4, 5, 6, 7):
        w[i].z -= 3
    report.append(replace(porch, w, old_faces))
    canopy = porch.copy()
    canopy.data = porch.data.copy()
    canopy.name = 'Lower Bailey Southeast Cottage / Porch canopy with closed overhang'
    bpy.data.collections['Derby Working'].objects.link(canopy)
    center = sum(top, Vector()) / 4
    for p in top:
        d = p - center
        d.z = 0
        p += d.normalized() * 1.2
    report.append(slab(canopy, top, 3))
    # The old tetrahedron extended past the gable as an unsupported large flap.
    # Its tower-hidden provenance supports no exposed architectural feature.
    # Retain the part as a small closed foundation course at the existing end.
    extension = next(o for o in objects if o['source_node'] == 'building-011')
    east = next(o for o in objects if o['source_node'] == 'building-060' and 'closed wall' in o.name)
    a, b = [world[east.name][i].copy() for i in (2, 3)]
    axis = (world[east.name][1] - a).normalized()
    top = [a, b, b + axis * 4, a + axis * 4]
    for p in top:
        p.z = 3
    report.append(slab(extension, top, 3))
    for obj in objects:
        obj['southeast_round2'] = True
    bpy.context.view_layer.update()
    return {'recipe': RECIPE, 'objects': report,
            'uncertainty': 'Tower-hidden gable/extension and doorway depth are conservative estimates; no invented rear detail',
            'source_mask': {'index': 0, 'layer': 0, 'use': 'reviewed exterior silhouette; gate tower 63 excluded'}}
