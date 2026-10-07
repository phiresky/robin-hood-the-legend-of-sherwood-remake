"""CPU geometry helpers for uncapped wood ownership and retained source islands."""
import math
import numpy as np

SIN = math.sin(math.radians(35))
COS = math.cos(math.radians(35))
RAY = np.array([0., -COS, SIN])


def projected(points):
    points = np.asarray(points)
    return np.column_stack((points[:, 0], -points[:, 1]*SIN-points[:, 2]*COS))


def source_pixel_fragment(triangle, pixel):
    """Clip an existing world-space triangle inside one source-pixel prism.

Every new point is an interpolation on the existing triangle. This cannot
create a new root bridge or move the observed surface along its source ray.
"""
    polygon = list(np.asarray(triangle, float))
    x, y = pixel
    for axis, bound, sign in [(0, x, 1), (0, x+1, -1), (1, y, 1), (1, y+1, -1)]:
        clipped = []
        for a, b in zip(polygon, polygon[1:]+polygon[:1]):
            qa, qb = projected([a, b])[:, axis]
            da, db = sign*(qa-bound), sign*(qb-bound)
            if da >= -1e-10:
                clipped.append(a)
            if (da >= 0) != (db >= 0):
                clipped.append(a+(b-a)*(da/(da-db)))
        polygon = clipped
        if len(polygon) < 3:
            return []
    return polygon


def owner32(center):
    """Native owner labels on a shared exterior; never add partition caps."""
    return 80 if center[2] >= 45 else (81 if center[0] >= 1072 else 82)


def support_sections(vertices, triangles, height=0.):
    """Measure actual triangle/receiver-plane intersections, not canopy bounds."""
    segments = []
    points = np.asarray(vertices)
    for triangle in np.asarray(triangles, int):
        face = points[triangle]
        crossings = []
        for a, b in zip(face, np.roll(face, -1, axis=0)):
            if (a[2]-height)*(b[2]-height) < 0:
                crossings.append(a+(b-a)*((height-a[2])/(b[2]-a[2])))
        if len(crossings) == 2:
            segments.append(crossings)
    if not segments:
        return dict(segments=0, plane_z=height, status='No measured contact section')
    p = np.asarray(segments).reshape(-1, 3)
    return dict(segments=len(segments), plane_z=height, world_xy_min=p[:, :2].min(axis=0).tolist(), world_xy_max=p[:, :2].max(axis=0).tolist(), section_length=float(np.linalg.norm(np.diff(np.asarray(segments), axis=1)[:, 0], axis=1).sum()), status='Geometric contact with this plane only; opaque source and actual receiver footprint still require validation')
