"""Correct the lying cask's flattened cross section and source-view alignment."""
import math

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-bailey-crate'
TAG = 'east-bailey-circular-cask-r2'
# A compact circular-cask fit to reviewed exterior mask 96. The silhouette
# constrains these five parameters, never individual vertices or hidden detail.
LENGTH_FACTOR = 1.0775221907802766
RADIUS = 10.72597800345678
SHIFT_XY = (0.5570258992458162, -2.399783161482061)
YAW = 0.384109452402841


def refine():
    bpy.context.view_layer.update()
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == 'building-112']
    if len(objects) != 3 or any(o.get('asset_group') != ASSET for o in objects):
        raise ValueError('Expected the owned barrel body and its two hoops')
    if all(o.get('round2_east_barrel') == TAG for o in objects):
        return {'asset': ASSET, 'status': 'already-refined'}
    body = next(o for o in objects if len(o.data.vertices) == 290)
    points = [body.matrix_world @ v.co for v in body.data.vertices]
    origin = (points[0] + points[-1]) / 2
    axis = points[-1] - points[0]
    axis.z = 0
    axis.normalize()
    side = Vector((-axis.y, axis.x, 0))
    radial_width = max(abs((p-origin).dot(side)) for p in points)
    radial_height = max(abs(p.z-origin.z) for p in points)
    new_axis = Vector((math.cos(YAW)*axis.x-math.sin(YAW)*axis.y,
                       math.sin(YAW)*axis.x+math.cos(YAW)*axis.y, 0))
    new_side = Vector((-new_axis.y, new_axis.x, 0))
    transformed = {}
    for obj in objects:
        output = []
        for vertex in obj.data.vertices:
            q = obj.matrix_world @ vertex.co - origin
            p = origin + new_axis * (q.dot(axis)*LENGTH_FACTOR)
            p += new_side * (q.dot(side)/radial_width*RADIUS)
            p.z = q.z/radial_height*RADIUS
            p.x += SHIFT_XY[0]
            p.y += SHIFT_XY[1]
            output.append(p)
        transformed[obj] = output
    lowest = min(p.z for points in transformed.values() for p in points)
    report = []
    for obj, points in transformed.items():
        # Preserve both hoop ground contacts. This courtyard's terrain is level
        # at z=0; the timber belly stays slightly above its supporting hoops.
        inverse = obj.matrix_world.inverted()
        obj.data = obj.data.copy()
        for vertex, point in zip(obj.data.vertices, points):
            point.z -= lowest
            vertex.co = inverse @ point
        obj.data.update()
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        defects = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
                   'degenerate_faces': sum(f.calc_area() < 1e-8 for f in bm.faces)}
        if any(defects.values()):
            bm.free()
            raise ValueError(f'{obj.name}: {defects}')
        bm.to_mesh(obj.data)
        bm.free()
        obj['round2_east_barrel'] = TAG
        report.append({'object': obj.name, **defects})
    bpy.context.view_layer.update()
    return {'asset': ASSET, 'status': 'refined', 'components': report,
            'old_radius_width': radial_width, 'old_radius_height': radial_height,
            'new_circular_radius': RADIUS, 'yaw_degrees': math.degrees(YAW),
            'length_factor': LENGTH_FACTOR, 'ground_z': 0,
            'reviewed_mask': 96, 'mask_state': 'exterior initial'}
