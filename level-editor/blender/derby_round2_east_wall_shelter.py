"""Planar eave reconstruction for the east-wall timber shelter.

Source anchors describe the exposed roof edge, not the foreground stair mask.
This recipe requires visual review and source reprojection before publication.
"""
import math

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-wall-landing'
TAG = 'round2-east-wall-shelter-eave-v1'


def _world(obj):
    return [obj.matrix_world @ v.co for v in obj.data.vertices]


def _check(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    report = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
              'degenerate_faces': sum(f.calc_area() < 1e-7 for f in bm.faces)}
    bm.to_mesh(obj.data)
    bm.free()
    if any(report.values()):
        raise RuntimeError(f'Invalid shelter geometry: {report}')
    return report


def refine(low_eave_z=70.0, near_pixel=(1570.0, 1466.0),
           far_pixel=(1597.0, 1413.0)):
    """Keep straight level eaves, one roof plane and constant vertical thickness.

    Parameters describe the source-supported exposed edge, kept level in 3D.
    The concealed uphill edge remains parallel to the exposed low eave; its
    nearer attachment is preserved. The wall tops follow the new underside,
    while their footprint and foundation remain unchanged.
    """
    owned = [o for o in bpy.data.collections['Derby Working'].objects
             if o.type == 'MESH' and not o.hide_render
             and o.get('asset_group') == ASSET
             and o.get('source_node') == 'building-069']
    if len(owned) != 2:
        raise RuntimeError('Expected exactly roof and body for shelter 069')
    roof = next(o for o in owned if o.get('shelter_component') == 'roof')
    body = next(o for o in owned if o.get('shelter_component') == 'body')
    if roof.get('round2_shelter') == TAG:
        return {'reused': True, 'tag': TAG}
    if any(len(o.data.vertices) != 8 for o in owned):
        raise RuntimeError('Expected audited eight-vertex shelter pieces')
    old = _world(roof)
    # Existing recipe stores top corners first, then corresponding underside.
    upper = old[:4]
    low = sorted(range(4), key=lambda i: upper[i].z)[:2]
    high = [i for i in range(4) if i not in low]
    near_low, far_low = sorted(low, key=lambda i: upper[i].x)
    near_high, far_high = sorted(high, key=lambda i: upper[i].x)
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def from_pixel(pixel):
        x, y = pixel
        return Vector((x, -(y + low_eave_z * cosine) / sine, low_eave_z))

    near, far = from_pixel(near_pixel), from_pixel(far_pixel)
    uphill = upper[near_high].copy()
    new = list(upper)
    new[near_low], new[far_low] = near, far
    new[near_high], new[far_high] = uphill, uphill + far - near
    normal = (far - near).cross(uphill - near)
    if abs(normal.z) < 1e-6:
        raise RuntimeError('Roof plane cannot define wall tops')
    thickness = sum(upper[i].z - old[i + 4].z for i in range(4)) / 4
    inverse = roof.matrix_world.inverted()
    for i, point in enumerate(new):
        roof.data.vertices[i].co = inverse @ point
        roof.data.vertices[i + 4].co = inverse @ (point - Vector((0, 0, thickness)))
    body_points = _world(body)
    inverse = body.matrix_world.inverted()
    for i, point in enumerate(body_points[:4]):
        point.z = (near.z - (normal.x * (point.x - near.x)
                             + normal.y * (point.y - near.y)) / normal.z
                   - thickness)
        if point.z <= body_points[i + 4].z:
            raise RuntimeError('Roof moved below shelter foundation')
        body.data.vertices[i].co = inverse @ point
    report = {'tag': TAG, 'source_node': 'building-069',
              'near_pixel': list(near_pixel), 'far_pixel': list(far_pixel),
              'roof_thickness': thickness, 'low_eave_z': low_eave_z,
              'validation': {o.name: _check(o) for o in owned}}
    for obj in owned:
        obj['round2_shelter'] = TAG
        obj.data.update()
    return report
