"""Pure-Python solid builders for the Lincoln west-complex refinement lane.

Every builder works in native map coordinates: x and y are map-plane pixels and
z is native height. A native point projects to source pixel (x, y - z). Blender
world coordinates are X = x, Y = -y / sin35, Z = z / cos35, so a world-space
circle of radius r has native half-axes (r, r * sin35).

Shapes are lists of vertices and polygon faces. Each builder returns closed
shells; ``Shape.add`` concatenates disjoint or touching shells and the Blender
side merges coincident vertices and recomputes outward normals.
"""
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REFINEMENT = ROOT / 'work/needs-review/lincoln-refinement'
SINE = math.sin(math.radians(35))
COSINE = math.cos(math.radians(35))
PLATEAU_Z = 220.0  # native top of the castle-hill plateau volumes (terrain lane keeps it fixed)


def native_obstacles():
    return json.loads((REFINEMENT / 'source-states/level.json').read_text())['sight_obstacles']


def points(obstacles, node):
    return [(p['x'], p['y'], p['z_bottom'], p['z_top']) for p in obstacles[node]['points']]


def pixel(x, y, z):
    return (x, y - z)


def to_world(p):
    x, y, z = p
    return (x, -y / SINE, z / COSINE)


class Shape:
    def __init__(self):
        self.verts, self.faces = [], []

    def add(self, other):
        base = len(self.verts)
        self.verts.extend(other.verts)
        self.faces.extend([[base + i for i in f] for f in other.faces])
        return self

    @staticmethod
    def of(verts, faces):
        s = Shape()
        s.verts = [tuple(float(c) for c in v) for v in verts]
        s.faces = [list(f) for f in faces]
        return s

    def world(self):
        return [to_world(v) for v in self.verts], self.faces


def _per_vertex(value, n):
    if isinstance(value, (int, float)):
        return [float(value)] * n
    if len(value) != n:
        raise ValueError('Per-vertex height list length mismatch')
    return [float(v) for v in value]


def signed_area(poly):
    return sum(poly[i][0] * poly[(i + 1) % len(poly)][1] - poly[(i + 1) % len(poly)][0] * poly[i][1]
               for i in range(len(poly))) / 2


def prism(poly, bottom, top):
    """Closed vertical prism over a simple native footprint polygon."""
    poly = [(float(x), float(y)) for x, y in poly]
    clean = []
    for p in poly:
        if not clean or math.dist(clean[-1], p) > 1e-6:
            clean.append(p)
    if math.dist(clean[0], clean[-1]) < 1e-6:
        clean.pop()
    poly = clean
    n = len(poly)
    if n < 3:
        raise ValueError('Degenerate prism footprint')
    zb, zt = _per_vertex(bottom, n), _per_vertex(top, n)
    if any(t - b <= 1e-6 for b, t in zip(zb, zt)):
        raise ValueError('Prism top must lie above bottom at every vertex')
    verts = [(x, y, z) for (x, y), z in zip(poly, zb)] + [(x, y, z) for (x, y), z in zip(poly, zt)]
    faces = [list(reversed(range(n))), list(range(n, 2 * n))]
    faces += [[i, (i + 1) % n, (i + 1) % n + n, i + n] for i in range(n)]
    return Shape.of(verts, faces)


def extrude_profile(edge_a, edge_b, profile):
    """Extrude a closed (t, z) profile between two native footprint edges.

    ``edge_a`` and ``edge_b`` are ((x0, y0), (x1, y1)); t = 0..1 runs along both
    edges. The profile is a simple polygon in the (t, z) plane, for example a
    crenellated wall elevation.
    """
    def at(edge, t):
        (x0, y0), (x1, y1) = edge
        return (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t)
    clean = []
    for p in profile:
        if not clean or math.dist(clean[-1], p) > 1e-9:
            clean.append(p)
    if math.dist(clean[0], clean[-1]) < 1e-9:
        clean.pop()
    n = len(clean)
    verts = [(*at(edge_a, t), z) for t, z in clean] + [(*at(edge_b, t), z) for t, z in clean]
    faces = [list(reversed(range(n))), list(range(n, 2 * n))]
    faces += [[i, (i + 1) % n, (i + 1) % n + n, i + n] for i in range(n)]
    return Shape.of(verts, faces)


def crenel_profile(bottom, top, floor, notches):
    """Closed (t, z) wall elevation with rectangular notches.

    ``bottom``, ``top`` (merlon cap) and ``floor`` (notch floor) are scalars or
    (z_at_t0, z_at_t1) pairs for sloping runs. ``notches`` are (t0, t1)
    intervals along the run; intervals may touch the run ends.
    """
    pair = lambda v: v if isinstance(v, tuple) else (v, v)
    (b0, b1), (c0, c1), (f0, f1) = pair(bottom), pair(top), pair(floor)
    cap = lambda t: c0 + (c1 - c0) * t
    low = lambda t: f0 + (f1 - f0) * t
    intervals = sorted((max(0.0, lo), min(1.0, hi)) for lo, hi in notches)
    for (a, b), (c, d) in zip(intervals, intervals[1:]):
        if c <= b:
            raise ValueError('Overlapping notches')
    upper = [(1.0, low(1.0) if intervals and intervals[-1][1] >= 1.0 else cap(1.0))]
    for lo, hi in reversed(intervals):
        if hi < 1.0:
            upper += [(hi, cap(hi)), (hi, low(hi))]
        if lo > 0.0:
            upper += [(lo, low(lo)), (lo, cap(lo))]
    upper.append((0.0, low(0.0) if intervals and intervals[0][0] <= 0.0 else cap(0.0)))
    return [(0.0, b0), (1.0, b1)] + upper


def lathe(center, profile, segments=48, start=0.0, end=math.tau):
    """Solid of revolution about a vertical native axis.

    ``profile`` is a closed simple polygon of (world_radius, native_z) points;
    points with radius 0 lie on the axis. A partial sweep (start..end) is closed
    with two radial end caps so each roof sector owner remains a closed shell.
    """
    cx, cy = center
    full = abs((end - start) - math.tau) < 1e-9
    count = segments if full else segments + 1
    angles = [start + (end - start) * i / segments for i in range(count)]
    verts, rings = [], []
    for r, z in profile:
        if r < 1e-9:
            rings.append([len(verts)] * count)
            verts.append((cx, cy, z))
        else:
            ids = []
            for a in angles:
                ids.append(len(verts))
                verts.append((cx + r * math.cos(a), cy + r * math.sin(a) * SINE, z))
            rings.append(ids)
    faces = []
    m = len(profile)
    for k in range(m):
        ra, rb = rings[k], rings[(k + 1) % m]
        steps = segments
        for i in range(steps):
            j = (i + 1) % count if full else i + 1
            quad = [ra[i], ra[j], rb[j], rb[i]]
            face = [v for idx, v in enumerate(quad) if v != quad[idx - 1]]
            if len(set(face)) >= 3:
                faces.append(face)
    if not full:
        faces.append([ring[0] for ring in rings])
        faces.append([ring[-1] for ring in reversed(rings)])
        # Axis points shared by both caps are single vertices; drop repeats.
        faces = [[v for idx, v in enumerate(f) if v != f[idx - 1]] for f in faces]
    return Shape.of(verts, faces)


def ellipse_ring(center, radius, z, segments=48, start=0.0, end=math.tau):
    cx, cy = center
    full = abs((end - start) - math.tau) < 1e-9
    count = segments if full else segments + 1
    return [(cx + radius * math.cos(start + (end - start) * i / segments),
             cy + radius * math.sin(start + (end - start) * i / segments) * SINE, z) for i in range(count)]


def fit_circle_native(pts):
    """Least-squares circle in world metric from native (x, y) samples."""
    import itertools
    xs = [p[0] for p in pts]
    ys = [p[1] / SINE for p in pts]
    n = len(pts)
    sx, sy = sum(xs), sum(ys)
    sxx = sum(x * x for x in xs); syy = sum(y * y for y in ys); sxy = sum(x * y for x, y in zip(xs, ys))
    sxz = sum(x * (x * x + y * y) for x, y in zip(xs, ys)); syz = sum(y * (x * x + y * y) for x, y in zip(xs, ys))
    sz = sum(x * x + y * y for x, y in zip(xs, ys))
    # Solve [sxx sxy sx; sxy syy sy; sx sy n] [a b c] = [sxz syz sz]
    m = [[sxx, sxy, sx, sxz], [sxy, syy, sy, syz], [sx, sy, n, sz]]
    for col in range(3):
        piv = max(range(col, 3), key=lambda r: abs(m[r][col]))
        m[col], m[piv] = m[piv], m[col]
        for r in range(3):
            if r != col:
                f = m[r][col] / m[col][col]
                m[r] = [a - f * b for a, b in zip(m[r], m[col])]
    a, b, c = (m[i][3] / m[i][i] for i in range(3))
    cx, cyw = a / 2, b / 2
    r = math.sqrt(c + cx * cx + cyw * cyw)
    return (cx, cyw * SINE), r
