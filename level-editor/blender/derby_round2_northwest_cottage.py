"""Give the northwest cottage coherent rounded thatch sections and a ridge cap."""
import math

import bpy
import bmesh
from mathutils import Vector

TAG = 'northwest-cottage-round2-rounded-thatch-v1'
NODES = ('building-065', 'building-066')


def _facade_relief(bm):
    """Shallow timbers on a single shared facade plane, from measured source lines.

    Screen points locate visible framing only; every point intersects the same
    vertical wall plane. Depth never comes from an independently fitted pixel.
    """
    a = Vector((515.581, -3238.669, 0))
    b = Vector((624.386, -3191.113, 0))
    tangent = (b-a).normalized()
    normal = Vector((tangent.y, -tangent.x, 0))

    def point(x, y):
        p = a.lerp(b, (x-a.x)/(b.x-a.x))
        p.z = (-p.y*.573576436351046-y)/.819152044288992
        if not 1 <= p.z <= 65:
            raise ValueError(f'Facade anchor outside supported wall: {x}, {y}: {p.z}')
        return p

    def beam(start, end, width, depth=1.0):
        p, q = point(*start), point(*end)
        axis = (q-p).normalized()
        side = normal.cross(axis).normalized()*(width/2)
        rings = [[bm.verts.new(r+s+normal*d) for r,s in
                  ((p,-side),(q,-side),(q,side),(p,side))]
                 for d in (-.15, depth)]
        bm.faces.new(rings[0][::-1]); bm.faces.new(rings[1])
        for i in range(4):
            bm.faces.new((rings[0][i],rings[0][(i+1)%4],
                          rings[1][(i+1)%4],rings[1][i]))

    # Uprights and braces follow the surviving front timber-frame artwork.
    for x, top, bottom, width in ((547,1802,1846,2.3),
                                 (580,1796,1839,2.0),
                                 (616,1788,1828,2.3)):
        beam((x,top),(x,bottom),width)
    beam((520,1828),(544,1845),1.5,.8)
    beam((549,1845),(576,1818),1.5,.8)
    beam((550,1819),(576,1812),1.6,.8)


def refine_facade():
    """Incremental facade pass; call on the validated rounded-roof candidate."""
    objects = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == 'building-066']
    if len(objects) != 1:
        raise ValueError('Expected one south facade source')
    obj = objects[0]
    if obj.get('northwest_facade_relief'):
        raise ValueError('Facade relief already applied')
    mesh = obj.data.copy(); obj.data = mesh
    bm = bmesh.new(); bm.from_mesh(mesh)
    _facade_relief(bm)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-7 for f in bm.faces)
    if bad_edges or bad_faces:
        raise ValueError(f'Facade topology: {bad_edges}, {bad_faces}')
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh); bm.free(); mesh.update()
    obj['northwest_facade_relief'] = True
    return {'source_node': 'building-066', 'nonmanifold_edges': bad_edges,
            'degenerate_faces': bad_faces, 'faces': len(mesh.polygons)}


def refine():
    objects = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') in NODES]
    if len(objects) != 2:
        raise ValueError('Expected exactly the two cottage half-shells')
    report = []
    for obj in objects:
        if obj.get('refinement_recipe') == TAG:
            raise ValueError('Recipe must run on the immutable baseline')
        mesh = obj.data.copy()
        obj.data = mesh
        bm = bmesh.new()
        bm.from_mesh(mesh)
        # The footprint, level ridge and straight eave rails are shared controls.
        # Subdivision adds a shallow transverse crown without independently
        # tracing either roof end or changing the wall planes underneath.
        top = [f for f in bm.faces if all(v.co.z > 69.9 for v in f.verts)]
        edges = list({e for f in top for e in f.edges})
        bmesh.ops.subdivide_edges(bm, edges=edges, cuts=7, use_grid_fill=True)
        for v in bm.verts:
            z = v.co.z
            if z >= 69.9:
                t = max(0.0, min(1.0, (z - 70.0) / 51.1))
                v.co.z += 3.0 * math.sin(math.pi * t)
        eaves = [e for e in bm.edges if all(abs(v.co.z-70.0) < .01 for v in e.verts)]
        bmesh.ops.bevel(bm, geom=eaves, offset=.8, segments=3,
                        affect='EDGES', clamp_overlap=True)
        # A small closed roll sits on the observed ridge, never above its ends
        # as independent spikes. It belongs to the north source part only.
        if obj['source_node'] == 'building-065':
            a = Vector((511.74, -3188.805, 120.75))
            b = Vector((596.237, -3151.918, 120.75))
            tangent = (b-a).normalized()
            across = Vector((-tangent.y, tangent.x, 0))
            rings = []
            for center, scale in ((a, .45), (a+tangent*2, 1),
                                  (b-tangent*2, 1), (b, .45)):
                rings.append([bm.verts.new(center + across*(2.4*scale*math.cos(i*math.tau/12))
                    + Vector((0, 0, 1.8*scale*math.sin(i*math.tau/12))))
                    for i in range(12)])
            bm.faces.new(rings[0][::-1])
            bm.faces.new(rings[-1])
            for low, high in zip(rings, rings[1:]):
                for i in range(12):
                    bm.faces.new((low[i], low[(i+1)%12], high[(i+1)%12], high[i]))
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        for f in bm.faces:
            f.smooth = f.normal.z > .1 and all(v.co.z >= 68.9 for v in f.verts)
        for e in bm.edges:
            if e.is_manifold:
                e.smooth = e.calc_face_angle() < .35
        bad_edges = sum(not e.is_manifold for e in bm.edges)
        bad_faces = sum(f.calc_area() < 1e-7 for f in bm.faces)
        if bad_edges or bad_faces:
            raise ValueError(f'{obj.name}: {bad_edges} open edges, {bad_faces} bad faces')
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        bm.to_mesh(mesh)
        bm.free()
        mesh.update()
        obj['refinement_recipe'] = TAG
        report.append({'source_node': obj['source_node'], 'vertices': len(mesh.vertices),
                       'faces': len(mesh.polygons), 'nonmanifold_edges': bad_edges,
                       'degenerate_faces': bad_faces})
    return report
