"""Measure the upper east curtain crown against covered artwork.

The source-facing vertical details below the crown are not automatically
crenel openings. Keep screen-space measurements explicit before changing their
depth or spacing. Conditional actor masks are supplementary silhouette evidence.
"""
import math

import bpy
import bmesh

ASSET = 'derby-upper-east-curtain'
PARTS = {'building-123', 'building-124'}


def _owned():
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and o.get('asset_group') == ASSET
               and not o.hide_render]
    if {o.get('source_node') for o in objects} != PARTS:
        raise ValueError('Upper east curtain ownership changed; re-audit parts')
    return objects


def _project(p):
    angle = math.radians(35)
    return [round(p.x, 4), round(-p.y * math.sin(angle)
                               - p.z * math.cos(angle), 4)]


def audit():
    """Return topology and crown measurements without changing the model."""
    bpy.context.view_layer.update()
    report = {'asset': ASSET, 'changed': False, 'parts': []}
    for obj in _owned():
        world = obj.matrix_world
        points = [world @ v.co for v in obj.data.vertices]
        top = max(p.z for p in points)
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        topology = {
            'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
            'degenerate_faces': sum(f.calc_area() < 1e-8 for f in bm.faces),
        }
        bm.free()
        # Horizontal ledges include notch bottoms and merlon tops. Separate
        # their planes rather than treating decorative painted joints as cuts.
        ledges = []
        for face in obj.data.polygons:
            p = [points[i] for i in face.vertices]
            if min(v.z for v in p) < top - 55:
                continue
            if max(v.z for v in p) - min(v.z for v in p) > 0.01:
                continue
            ledges.append({
                'face': face.index,
                'z': round(sum(v.z for v in p) / len(p), 4),
                'source_polygon': [_project(v) for v in p],
            })
        report['parts'].append({
            'name': obj.name, 'source_node': obj['source_node'],
            'vertices': len(points), 'faces': len(obj.data.polygons),
            'top_z': top, 'bottom_z': min(p.z for p in points),
            'topology': topology, 'horizontal_crown_faces': ledges,
            'crown_vertices': [
                {'vertex': i, 'world': list(p), 'source': _project(p)}
                for i, p in enumerate(points) if p.z > top - 55
            ],
        })
    return report
