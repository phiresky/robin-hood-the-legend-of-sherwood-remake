"""Review and restore a shallow front eave on the north curtain timber shed.

The roof remains an unchanged architectural plane. Recessing its supporting
wall produces the visible board-end lip without tracing individual source
pixels. Run inspect() first; refine() requires an explicitly measured setback.
"""
import bpy
from mathutils import Vector

ASSET = 'derby-east-courtyard-north-shelter'
TAG = 'round2_north_shelter_eave'


def _components():
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render and
               o.get('asset_group') == ASSET]
    if len(objects) != 5:
        raise ValueError(f'Expected five active shed components, got {len(objects)}')
    return objects


def _pair(objects, node):
    owned = [o for o in objects if o.get('source_node') == node]
    roofs = [o for o in owned if 'roof' in o.name.lower() and
             'walls' not in o.name.lower().rsplit(' / ', 1)[-1]]
    walls = [o for o in owned if 'walls' in o.name.lower().rsplit(' / ', 1)[-1]]
    if len(roofs) != 1 or len(walls) != 1:
        raise ValueError(f'Expected roof and walls for {node}')
    return roofs[0], walls[0]


def _frame(roof, walls):
    points = [roof.matrix_world @ v.co for v in roof.data.vertices]
    candidates = []
    for poly in roof.data.polygons:
        a, b, c = (points[i] for i in poly.vertices[:3])
        normal = (b-a).cross(c-a)
        if normal.length < 1e-8:
            continue
        normal.normalize()
        if normal.z > .3:
            candidates.append((poly.area, normal, a))
    if not candidates:
        raise ValueError('No upward roof plane')
    _, normal, origin = max(candidates, key=lambda item: item[0])
    downhill = Vector((normal.x, normal.y, 0))
    if downhill.length < .05:
        raise ValueError('Cannot infer front eave from nearly flat roof')
    downhill.normalize()
    distances = [normal.dot(p-origin) for p in points]
    lower = min(distances)
    if max(abs(d) if abs(d) < abs(d-lower) else abs(d-lower)
           for d in distances) > .02:
        raise ValueError('Roof must consist of two parallel planes')
    bottom_origin = origin + normal * lower
    world_wall = [walls.matrix_world @ v.co for v in walls.data.vertices]
    back = min(p.dot(downhill) for p in world_wall)
    front = max(p.dot(downhill) for p in world_wall)
    roof_front = max(p.dot(downhill) for p in points)
    return normal, bottom_origin, downhill, back, front, roof_front


def inspect():
    result = {'asset': ASSET, 'components': [], 'eaves': {}}
    objects = _components()
    for obj in objects:
        points = [obj.matrix_world @ v.co for v in obj.data.vertices]
        result['components'].append({
            'name': obj.name, 'source_node': obj.get('source_node'),
            'vertices': len(points), 'faces': len(obj.data.polygons),
            'bounds': [[min(p[i] for p in points) for i in range(3)],
                       [max(p[i] for p in points) for i in range(3)]]})
    for node in ('building-071', 'building-072'):
        roof, wall = _pair(objects, node)
        normal, origin, direction, back, front, roof_front = _frame(roof, wall)
        result['eaves'][node] = {
            'roof': roof.name, 'wall': wall.name,
            'downhill_direction': list(direction), 'roof_normal': list(normal),
            'wall_depth': front-back, 'existing_front_overhang': roof_front-front,
            'underside_plane_origin': list(origin),
            'wall_roof_contacts': sum(abs(normal.dot(wall.matrix_world@v.co-origin))
                                      < .03 for v in wall.data.vertices)}
    return result


def refine(*, setback_by_node):
    """Set desired front overhang in map units; zero/absent nodes stay unchanged."""
    if set(setback_by_node) - {'building-071', 'building-072'}:
        raise ValueError('Only annex and main supporting walls can be adjusted')
    objects = _components()
    plans = []
    for node, desired in setback_by_node.items():
        if not 0 < desired <= 3:
            raise ValueError('Reviewed shallow eave must be between zero and three units')
        roof, wall = _pair(objects, node)
        if wall.get(TAG) is not None:
            if abs(float(wall[TAG])-desired) > 1e-6:
                raise ValueError('Reload baseline before changing eave parameters')
            continue
        normal, origin, direction, back, front, roof_front = _frame(roof, wall)
        delta = desired-(roof_front-front)
        depth = front-back
        if delta <= 0 or delta >= depth*.08:
            raise ValueError(f'Unsupported wall setback {delta} at {node}')
        inv = wall.matrix_world.inverted()
        points = []
        for vertex in wall.data.vertices:
            p = wall.matrix_world @ vertex.co
            on_roof = abs(normal.dot(p-origin)) < .03
            p -= direction * (delta * (p.dot(direction)-back)/depth)
            if on_roof:
                p.z -= normal.dot(p-origin)/normal.z
            points.append(inv @ p)
        plans.append((wall, desired, points))
    for wall, desired, points in plans:
        wall.data = wall.data.copy()
        for vertex, point in zip(wall.data.vertices, points):
            vertex.co = point
        wall.data.update()
        wall[TAG] = desired
        wall['refinement_recipe'] = 'round2-north-shelter-coherent-front-eave-v1'
    bpy.context.view_layer.update()
    return {'changed': [wall.name for wall, _, _ in plans], 'inspection': inspect()}
