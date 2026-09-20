"""Small reviewed silhouette corrections for the south-curtain lean-to.

The curtain hides most walls: only explicitly reviewed roof columns and chimney
rim vertices may move. Source-pixel corrections change height, preserving the
footprint, roof thickness and coincident roof/support contacts.
"""
import math

import bpy
import bmesh
from mathutils import Vector

ASSET = 'derby-east-courtyard-south-shelter'
NODES = {'building-067', 'building-068'}
TAG = 'south-shelter-round2-reviewed-boundary-v1'
COS = math.cos(math.radians(35))
SIN = math.sin(math.radians(35))


def _objects():
    objects = [o for o in bpy.data.collections['Derby Working'].objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('asset_group') == ASSET]
    if len(objects) != 3 or {o.get('source_node') for o in objects} != NODES:
        raise ValueError('Expected roof, supporting shell and chimney for parts 67/68')
    return objects


def inspect():
    """Return small actual geometry tables before approving explicit offsets."""
    return [{'name': o.name, 'source_node': o['source_node'],
             'vertices': [{'index': v.index, 'world': list(p),
                           'source': [p.x, -p.y * SIN - p.z * COS]}
                          for v in o.data.vertices
                          for p in [o.matrix_world @ v.co]]}
            for o in _objects()]


def _validate(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    result = {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
              'degenerate_faces': sum(f.calc_area() < 1e-8 for f in bm.faces)}
    bm.free()
    if any(result.values()):
        raise ValueError(f'{obj.name}: invalid changed geometry {result}')
    return result


def _capped_chimney(obj):
    """Replace the erroneous open top with the visible side-vented cap."""
    world = [obj.matrix_world @ v.co for v in obj.data.vertices]
    vertices = [p.copy() for p in world[:4]] + [p.copy() for p in world[12:16]]
    # The source's dark aperture is below the cap on the source-facing wall.
    # Depth is a conservative estimate; aperture location is source-supported.
    a, b = vertices[:2]
    for u, drop in ((.20, 3), (.65, 3), (.65, 11), (.20, 11)):
        vertices.append(a.lerp(b, u) - Vector((0, 0, drop)))
    center = sum(vertices[:4], Vector()) / 4
    inward = center - (a + b) / 2
    inward.z = 0
    inward.normalize()
    vertices += [p + inward * 2 for p in vertices[8:12]]
    faces = [(0, 1, 2, 3), (7, 6, 5, 4),
             (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0),
             (0, 8, 9, 1), (1, 9, 10, 5),
             (5, 10, 11, 4), (4, 11, 8, 0), (12, 13, 14, 15)]
    faces += [(8+i, 12+i, 12+(i+1)%4, 8+(i+1)%4) for i in range(4)]
    mesh = bpy.data.meshes.new('South curtain chimney / closed cap and side vent')
    inv = obj.matrix_world.inverted()
    mesh.from_pydata([inv @ p for p in vertices], [], faces)
    for material in obj.data.materials:
        mesh.materials.append(material)
    # Reprojection regenerates all surface UVs after this topology replacement.
    mesh.uv_layers.new(name='UVMap')
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh)
    bm.free()
    obj.data = mesh
    obj['round2_south_shelter'] = TAG
    return {'object': obj.name, 'change': 'Closed masonry cap with recessed front vent',
            'validation': _validate(obj)}


def refine(*, roof_columns=None, chimney_vertices=(), close_chimney=True):
    """Apply reviewed `(world_x, world_y, source_dy)` roof-column offsets.

    Roof top, underside, and wall top sharing that column receive the identical
    height delta; ground vertices never move. Chimney entries are `(index, dy)`
    selected from inspect(), allowing its hollow rim to follow the painted cap.
    Positive dy moves downward in the source image. Inputs are deliberately
    explicit: a mask bbox cannot distinguish exposed eaves from curtain overlap.
    """
    if roof_columns is None:
        # Uniform one-source-pixel roof inset preserves the planar monopitch,
        # level long eaves, 2.5-unit slab and all roof/support junctions.
        roof_columns = ((1438.068359375, -3095.0703125, 1),
                        (1542.11328125, -2947.583984375, 1),
                        (1474.5684814453125, -2899.9111328125, 1),
                        (1370.523681640625, -3047.397216796875, 1))
    if not roof_columns and not chimney_vertices and not close_chimney:
        raise ValueError('No reviewed corrections supplied; inspect source evidence first')
    objects = _objects()
    if any(o.get('round2_south_shelter') == TAG for o in objects):
        raise ValueError('Already refined; reload immutable baseline before rerunning')
    edits = {}
    for x, y, dy in roof_columns:
        if abs(dy) > 3:
            raise ValueError('Roof change exceeds reviewed three-source-pixel boundary band')
        found = []
        for o in objects:
            if o['source_node'] != 'building-068':
                continue
            for v in o.data.vertices:
                p = o.matrix_world @ v.co
                if abs(p.x-x) < .01 and abs(p.y-y) < .01 and p.z > 10:
                    found.append((o, v.index))
        if len(found) != 3:
            raise ValueError(f'Expected roof top/underside/support contact at {(x,y)}, got {len(found)}')
        for o, index in found:
            edits.setdefault(o, {})[index] = -dy / COS
    chimney = next(o for o in objects if o['source_node'] == 'building-067')
    for index, dy in chimney_vertices:
        if abs(dy) > 3 or index < 0 or index >= len(chimney.data.vertices):
            raise ValueError('Invalid chimney boundary correction')
        p = chimney.matrix_world @ chimney.data.vertices[index].co
        if p.z < 150:
            raise ValueError('Chimney correction is outside the cap region')
        edits.setdefault(chimney, {})[index] = -dy / COS
    report = []
    for o, changes in edits.items():
        o.data = o.data.copy()
        inv = o.matrix_world.inverted()
        for index, dz in changes.items():
            p = o.matrix_world @ o.data.vertices[index].co
            o.data.vertices[index].co = inv @ (p + Vector((0, 0, dz)))
        o.data.update()
        o['round2_south_shelter'] = TAG
        report.append({'object': o.name, 'changes_z': changes,
                       'validation': _validate(o)})
    if close_chimney:
        report.append(_capped_chimney(chimney))
    bpy.context.view_layer.update()
    return {'recipe': TAG, 'objects': report,
            'scope': 'Only reviewed exposed roof columns and chimney cap; ground footprint retained'}
