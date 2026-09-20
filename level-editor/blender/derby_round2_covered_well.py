"""Repair covered-well joints and its source-supported shallow masonry foot."""
import math
import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-bailey-well'
TAG = 'covered-well-round2-v1'


def refine():
    objects = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and not o.hide_render and o.get('asset_group') == ASSET]
    if len(objects) != 4:
        raise ValueError('Expected two canopies, shaft and separate bucket')
    if all(o.get('round2_recipe') == TAG for o in objects):
        return {'status': 'already-refined'}
    canopies = {o['source_node']: o for o in objects if o['source_node'] != 'building-111'}
    shaft = max((o for o in objects if o['source_node'] == 'building-111'),
                key=lambda o: max((o.matrix_world @ v.co).x for v in o.data.vertices)
                - min((o.matrix_world @ v.co).x for v in o.data.vertices))
    bucket = next(o for o in objects if o['source_node'] == 'building-111' and o != shaft)
    # Match the shared ridge endpoints; the two inherited slabs differed by .2
    # vertically and several hundredths horizontally, exposing a thin crack.
    east, west = canopies['building-109'], canopies['building-110']
    if len(east.data.vertices) != 40 or len(west.data.vertices) != 40:
        raise ValueError('Unexpected canopy topology')
    for obj in (east, west):
        obj.data = obj.data.copy()
    for wi, ei in ((0, 2), (1, 1), (7, 5), (6, 6), (8, 8), (11, 11),
                   (12, 12), (15, 15), (24, 24), (27, 27), (28, 28), (31, 31)):
        west.data.vertices[wi].co = west.matrix_world.inverted() @ (east.matrix_world @ east.data.vertices[ei].co)
    # The gable top sat .02 below the roof underside. Close that slit with a
    # .03 overlap, far below a source pixel and without moving the roof outline.
    for obj in (east, west):
        for index in (8, 9, 14, 15, 24, 25, 30, 31):
            p = obj.matrix_world @ obj.data.vertices[index].co
            p.z += .05
            obj.data.vertices[index].co = obj.matrix_world.inverted() @ p
        obj.data.update()
    # Visible foot offsets measured against mask 94. Hidden half remains the
    # established circular radius; no arbitrary unseen stone ornament is added.
    offsets = [0.] * 32
    offsets[17:] = [2.5, 3., 2.75, 3.5, 0., .0, 2.75, 3.5, 3.5, 3.5, 3.5, 2.75, 1., 3., 2.]
    levels = [(0., .97), (4.5, .976), (21.5, 1.), (23.6, 1.015), (23.6, .76), (2., .76)]
    points = []
    for j, (z, scale) in enumerate(levels):
        for i in range(32):
            theta = i * math.tau / 32
            extra = offsets[i] if j == 0 else 0.
            p = Vector((1455.3 + (26.3 * scale + extra) * math.cos(theta),
                        -2595.5 + (26.5 * scale + extra) * math.sin(theta), z))
            points.append(shaft.matrix_world.inverted() @ p)
    faces = []
    for j in range(len(levels) - 1):
        for i in range(32):
            k = (i + 1) % 32
            faces.append((j*32+i, j*32+k, (j+1)*32+k, (j+1)*32+i))
    faces += [tuple(reversed(range(32))), tuple(range(160, 192))]
    mesh = bpy.data.meshes.new('Covered well / shallow masonry foot')
    mesh.from_pydata(points, [], faces)
    for mat in shaft.data.materials:
        mesh.materials.append(mat)
    mesh.uv_layers.new(name='UVMap')
    bm = bmesh.new(); bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces)); bm.to_mesh(mesh); bm.free()
    shaft.data = mesh
    shaft['projection_min_cosine'] = .05
    report = []
    for obj in objects:
        bm = bmesh.new(); bm.from_mesh(obj.data)
        bad = sum(not e.is_manifold for e in bm.edges)
        zero = sum(f.calc_area() < 1e-7 for f in bm.faces)
        bm.free()
        if bad or zero:
            raise ValueError(f'{obj.name}: {bad} nonmanifold, {zero} degenerate')
        obj['round2_recipe'] = TAG
        report.append({'name': obj.name, 'source_node': obj['source_node'],
                       'faces': len(obj.data.polygons), 'nonmanifold': bad, 'degenerate': zero,
                       'bucket_geometry_unchanged': obj == bucket})
    return {'objects': report, 'shaft_projection_min_cosine': .05}
