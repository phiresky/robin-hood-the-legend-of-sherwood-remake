"""Source-traced rock masses on Derby's outer banks.

The artwork establishes locations and projected extents, not surveyed heights.
Relief is explicitly inferred, bounded, and displaced along source-camera rays.
Navigation and structure support polygons remain untouched.
"""
import json
import math
from pathlib import Path

import numpy as np

TAG = "derby-rock-masses-v2"

# Center, projected half extents, game-height relief and plan rotation. These
# broad exposed rock masses exclude courtyard paths and atmospheric cloud bands.
ROCKS = (
    (116, 1225, 90, 106, 24, -18),
    (65, 1390, 72, 110, 27, 12),
    (194, 1523, 76, 91, 32, -12),
    (96, 1704, 78, 112, 29, 18),
    (62, 1886, 66, 110, 26, -16),
    (154, 2051, 89, 104, 32, 15),
    (66, 2208, 66, 100, 30, -20),
    (228, 2360, 100, 95, 29, -10),
    (128, 2514, 88, 103, 34, 16),
    (296, 2660, 85, 81, 23, -12),
    (1298, 1841, 92, 100, 28, -18),
    (1383, 1978, 90, 104, 31, 20),
    (1495, 2110, 91, 106, 33, -12),
    (1460, 2290, 80, 101, 29, 14),
    (1540, 2454, 86, 115, 31, -16),
    (1514, 2613, 95, 102, 25, 12),
)


def relief(x, y):
    """Irregular octagonal shoulders with broad tops and distinct side slopes."""
    result = np.zeros_like(x)
    for cx, cy, rx, ry, height, degrees in ROCKS:
        angle = math.radians(degrees)
        dx, dy = x-cx, y-cy
        u = (dx*math.cos(angle) + dy*math.sin(angle))/rx
        v = (-dx*math.sin(angle) + dy*math.cos(angle))/ry
        radius = np.maximum.reduce((np.abs(u), np.abs(v),
                                    np.abs(u+v)/1.48, np.abs(u-v)/1.37))
        shoulder = np.clip((1-radius)/.72, 0, 1)
        # A sloping crown avoids a pointed cone or a rectangular flat plateau.
        crown = np.clip(1-.16*u-.11*v, .7, 1.2)
        result = np.maximum(result, height*shoulder*crown)
    return result


def refine(level_path=None):
    import bpy
    from mathutils import Vector
    from derby_terrain import terrain_height, SIN, COS, ROOT

    level_path = Path(level_path or ROOT.parent / "datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.json")
    level = json.loads(level_path.read_text())
    grounds = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and o.get('source_node') == 'ground' and not o.hide_render]
    if len(grounds) != 1:
        raise ValueError('Expected exactly one visible ground surface')
    obj = grounds[0]
    mesh = obj.data
    baseline = mesh.attributes.get('rock_relief_baseline_world')
    if baseline is None:
        baseline = mesh.attributes.new('rock_relief_baseline_world', 'FLOAT_VECTOR', 'POINT')
        for vertex, item in zip(mesh.vertices, baseline.data):
            item.vector = obj.matrix_world @ vertex.co
    points = np.asarray([item.vector[:] for item in baseline.data], dtype=np.float64)
    x = points[:, 0]
    y = -points[:, 1]*SIN-points[:, 2]*COS
    _, supported = terrain_height(x, y, level, margin=24)
    delta = relief(x, y)
    # Blend out near the existing measured plateau; the local rock crowns may
    # not raise the castle foundation or produce a ridge through a road.
    game_height = points[:, 2]*COS
    delta *= np.clip(-game_height/35, 0, 1)
    delta[supported] = 0
    inverse = obj.matrix_world.inverted()
    for vertex, point, height in zip(mesh.vertices, points, delta):
        moved = Vector((point[0], point[1]-height/SIN, point[2]+height/COS))
        vertex.co = inverse @ moved
    mesh.update()
    after = np.asarray([obj.matrix_world @ v.co for v in mesh.vertices])
    error = np.max(np.abs(-after[:, 1]*SIN-after[:, 2]*COS-y))
    if error > .001:
        raise AssertionError(f'Source projection drift: {error}')
    # This recipe operates on the existing regular source grid. Monotone depth
    # prevents new front/back folds which would break inverse projection.
    nx = int(np.count_nonzero(np.isclose(y, y[0], atol=.001)))
    if nx < 2 or len(points) % nx:
        raise ValueError('Expected the regular source-camera ground grid')
    minimum_step = float(np.diff((-after[:, 1]*SIN).reshape(-1, nx), axis=0).min())
    if minimum_step <= 0:
        raise AssertionError(f'Terrain folds in depth: {minimum_step}')
    if any(p.area < 1e-8 for p in mesh.polygons):
        raise AssertionError('Degenerate terrain face')
    obj['terrain_detail_recipe'] = TAG
    obj['terrain_detail_evidence'] = 'Sixteen source-traced outer-bank rock masses; heights inferred, measured support polygons fixed'
    return dict(recipe=TAG, rock_masses=len(ROCKS), changed_vertices=int((delta>0).sum()),
                maximum_game_height_delta=float(delta.max()), source_projection_error=float(error),
                supported_displacement=float(np.max(np.abs(delta[supported]))),
                minimum_depth_step=minimum_step,
                limitations=['Rock depth is inferred from a single view.',
                             'Fine individual stone chips and distant trees remain painted detail.'])
