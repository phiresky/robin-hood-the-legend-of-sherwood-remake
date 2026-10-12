"""Lincoln ``east_gate_walls`` lane: measured geometry recipes.

Run on one isolated asset workspace (never on a frozen baseline):

  /usr/bin/blender --background <asset>/model.blend --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/refine_east_gate.py -- --asset <asset-id>

The recipe rebuilds only the owned meshes from the immutable ``baseline.blend``
of the same workspace, so repeated runs are idempotent.  It then saves
``model.blend``, writes ``inspection/geometry-recipe.json`` and
``inspection/actual-native.json`` (saved-mesh vertices in native units) and
regenerates the frozen-camera ``modified/`` packet with the pinned tooling.
Pass ``--no-packet`` to skip the packet (geometry iteration only).

Measurements are original source pixels of ``source-states/covered.png``.
Native units: pixel = (x, y - z).  The plateau datum is native z = 220.
Evidence (traces, overlays) lives in each workspace's ``inspection/``.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import east_gate_walls_geom as G  # noqa: E402

TOOLING = ROOT / 'level-editor/work/needs-review/lincoln-refinement/tooling/e6b57cb851c7142b'
PLATEAU = 220.0

# --------------------------------------------------------------------------
# Measured specifications
# --------------------------------------------------------------------------

# Round gate towers.  Centre x and radii come from native silhouette masks
# (235: north, 233: south); centre y from the authored parapet obstacle
# footprint (94/97), which fits the painted ellipse.  Merlon tops come from
# the far-merlon silhouette row (north 970-980 px, south 1122-1127 px), the
# parapet band bottom from the width step of the silhouette at phi = 0, the
# walkway floor from the authored body top.  Merlon centres were measured on
# the far arc (mask runs at rows 980/990 and 1132/1142) and checked against
# the painted near arc.
ROUND_TOWERS = {
    'lincoln-east-gate-north-tower': {
        'body': 78, 'ring': 94, 'cx': 2365.0, 'cy': 1481.5,
        'r_parapet': 78.0, 'r_body': 69.0, 'r_inner': 66.0,
        'ground': PLATEAU, 'parapet_bottom': 422.0, 'floor': 438.0,
        'crenel_floor': 450.0, 'merlon_top': 462.0,
        'merlons': {'count': 17, 'phase_deg': 11.5, 'width_deg': 12.5},
        'far_merlon_centres_px': [2308, 2331.5, 2359, 2387.5, 2413, 2433],
    },
    'lincoln-east-gate-south-tower': {
        'body': 80, 'ring': 97, 'cx': 2501.0, 'cy': 1629.0,
        'r_parapet': 78.5, 'r_body': 65.0, 'r_inner': 66.5,
        # Painted footing on the rock slope (mask 233 bottom row 1472 px).
        'ground': 195.0, 'parapet_bottom': 420.0, 'floor': 438.0,
        'crenel_floor': 449.0, 'merlon_top': 460.0,
        'merlons': {'count': 17, 'phase_deg': 11.3, 'width_deg': 12.5},
        'far_merlon_centres_px': [2443, 2467.5, 2495, 2523.5, 2549],
    },
}

# Gate block: parallelogram of authored piers 81/82 and walk slab 79.
# s runs along the courtyard (south-west) face from the north tower (s=0) to
# the south tower (s=1); d runs through the wall towards the outside.
GATE = {
    'p1': (2359.4, 1526.0), 'p2': (2437.6, 1606.3), 'p4': (2429.3, 1503.6),
    # Painted arch: jambs at 2380-2383 / 2418-2427 px, soffit 1265 px at s=0.525.
    'opening': (0.30, 0.75), 'spring': 273.5, 'crown': 303.0,
    'slab_bottom': 305.0, 'walk': 360.0,
    # Parapet profile along 95 (outer face), bright notch runs at z 364-372.
    'parapet_floor': 364.0, 'parapet_top': 374.0,
    'notches': [(0.045, 0.09), (0.185, 0.225), (0.31, 0.355), (0.44, 0.485),
                (0.57, 0.615), (0.70, 0.745), (0.83, 0.875)],
}

# Lower east curtain outer parapet (obstacle 98).  Merlon south-west side
# faces are shadowed blobs detected along the slab mid-line (x centroids).
LOWER = {
    'outer': ((2497.4, 1667.6), (2384.9, 1813.6)),
    'inner': ((2489.8, 1665.6), (2377.3, 1811.7)),
    'mid_x0': 2493.6, 'mid_dx': 112.5,
    'blob_x': [2489.8, 2479.3, 2470.5, 2460.4, 2450.4, 2440.8, 2430.8,
               2420.4, 2410.3, 2399.8, 2390.5],
    'notch_width_t': 0.036,
    'crenel_floor': 369.0, 'merlon_top': 380.0,
    # Footing from the bottom row of mask 232 along the outer face.
    'footing': (212.0, 168.0),
}


# --------------------------------------------------------------------------
# Builders (pure Python; ``base`` maps node -> native baseline mesh)
# --------------------------------------------------------------------------

def build_round_tower(spec, base):
    m = spec['merlons']
    merlons = G.merlon_intervals(m['phase_deg'], 360.0 / m['count'], m['width_deg'], m['count'])
    body = G.stepped_cylinder(spec['cx'], spec['cy'], [
        (spec['r_body'], spec['ground'], spec['parapet_bottom']),
        (spec['r_inner'] - 0.5, spec['parapet_bottom'], spec['floor'])], segments=48)
    ring = G.crenellated_ring(spec['cx'], spec['cy'], spec['r_parapet'], spec['r_inner'],
                              spec['parapet_bottom'], spec['crenel_floor'], spec['merlon_top'],
                              merlons, step_deg=7.5)
    return {spec['body']: (body, 'Measured Round Body'),
            spec['ring']: (ring, 'Measured Crenellated Parapet')}, {
        'merlons': m['count'], 'merlon_phase_deg': m['phase_deg'],
        'ground_z': spec['ground']}


def _solve_sd(p1, u, w, p):
    det = u[0] * w[1] - u[1] * w[0]
    dx, dy = p[0] - p1[0], p[1] - p1[1]
    return ((dx * w[1] - dy * w[0]) / det, (u[0] * dy - u[1] * dx) / det)


def build_gate_arch(base):
    g = GATE
    p1 = g['p1']
    u = (g['p2'][0] - p1[0], g['p2'][1] - p1[1])
    w = (g['p4'][0] - p1[0], g['p4'][1] - p1[1])
    s0, s1 = g['opening']
    sm = (s0 + s1) / 2
    arch = G.arch_profile(s0, s1, PLATEAU, g['spring'], g['crown'], segments=16)
    left = [p for p in arch if p[0] <= sm + 1e-9]
    right = [p for p in arch if p[0] >= sm - 1e-9]
    top = g['slab_bottom']
    north = [(0.0, PLATEAU)] + left + [(sm, top), (0.0, top)]
    south = [(sm, top)] + right + [(1.0, PLATEAU), (1.0, top)]
    out = {81: (G.extrude_profile(north, p1, u, w, (0.0, 1.0)), 'Measured Arch Pier'),
           82: (G.extrude_profile(south, p1, u, w, (0.0, 1.0)), 'Measured Arch Pier')}
    slab = G.Shell()
    corners = [p1, g['p2'], (p1[0] + u[0] + w[0], p1[1] + u[1] + w[1]), g['p4']]
    slab.prism(corners, top, g['walk'])
    out[79] = (slab, 'Measured Wall Walk')
    # Gate leaf 139: authored depth band, shaped to the measured opening.
    door = base[139]
    ds = [_solve_sd(p1, u, w, (x, y))[1] for x, y, z in door['verts']]
    leaf = [(s0 + 0.004, PLATEAU)] + [(max(s0 + 0.004, min(s1 - 0.004, s)), min(z, g['crown'] - 1.0))
                                      for s, z in arch[1:-1]] + [(s1 - 0.004, PLATEAU)]
    out[139] = (G.extrude_profile(leaf, p1, u, w, (min(ds), max(ds))), 'Measured Gate Leaf')
    # Parapets: authored thin footprints, crenellated from the walk level.
    for node, flip in ((95, False), (96, True)):
        v = base[node]['verts']
        pts = sorted({(round(x, 1), round(y, 1)) for x, y, z in v})
        sd = [(_solve_sd(p1, u, w, p), p) for p in pts]
        dvals = sorted(d for (s, d), p in sd)
        d_lo, d_hi = dvals[0], dvals[-1]
        svals = sorted(s for (s, d), p in sd)
        sa, sb = svals[0], svals[-1]
        def at(s, d):
            return (p1[0] + s * u[0] + d * w[0], p1[1] + s * u[1] + d * w[1])
        outer = (at(sa, d_hi if flip else d_lo), at(sb, d_hi if flip else d_lo))
        inner = (at(sa, d_lo if flip else d_hi), at(sb, d_lo if flip else d_hi))
        notches = [((a - sa) / (sb - sa), (b - sa) / (sb - sa)) for a, b in g['notches']]
        out[node] = (G.crenellated_wall(outer, inner, g['walk'], g['parapet_floor'],
                                        g['parapet_top'], notches), 'Measured Crenellated Parapet')
    return out, {'opening_s': g['opening'], 'arch_crown_z': g['crown'], 'walk_z': g['walk'],
                 'parapet_notches': len(g['notches']), 'ground_z': PLATEAU}


def build_lower_curtain(base):
    c = LOWER
    notches = []
    for x in c['blob_x']:
        t = (c['mid_x0'] - x) / c['mid_dx']
        notches.append((t, min(t + c['notch_width_t'], 0.999)))
    shell = G.crenellated_wall(c['outer'], c['inner'], c['footing'], c['crenel_floor'],
                               c['merlon_top'], notches)
    return {98: (shell, 'Measured Crenellated Parapet')}, {
        'notches': len(notches), 'ground_z': list(c['footing'])}


# --------------------------------------------------------------------------
# Generic helpers for authored thin parapets and datum pillars
# --------------------------------------------------------------------------

def footprint_corners(data, zmax=None):
    """Distinct ground corners of an authored prism (welded to 0.5 units)."""
    pts = []
    for x, y, z in data['verts']:
        if zmax is not None and z > zmax:
            continue
        if not any(abs(x - p[0]) < 0.6 and abs(y - p[1]) < 0.6 for p in pts):
            pts.append((x, y))
    return pts


def quad_lines(data):
    """Two long edges of a thin (possibly bevelled) footprint, ordered along the run.

    The run axis joins the farthest pair of corners; each end keeps its
    extreme perpendicular offsets so the full authored thickness is retained.
    """
    pts = footprint_corners(data)
    i, j = max(((i, j) for i in range(len(pts)) for j in range(i + 1, len(pts))),
               key=lambda ij: math.dist(pts[ij[0]], pts[ij[1]]))
    a, b = pts[i], pts[j]
    horizontal = abs(b[0] - a[0]) >= abs(b[1] - a[1])
    if (a[0] if horizontal else a[1]) > (b[0] if horizontal else b[1]):
        a, b = b, a
    L = math.dist(a, b)
    ux, uy = (b[0] - a[0]) / L, (b[1] - a[1]) / L
    along = lambda p: (p[0] - a[0]) * ux + (p[1] - a[1]) * uy
    perp = lambda p: -(p[0] - a[0]) * uy + (p[1] - a[1]) * ux
    lo_end = [p for p in pts if along(p) < L / 2]
    hi_end = [p for p in pts if along(p) >= L / 2]
    s0, s1 = min(along(p) for p in lo_end), max(along(p) for p in hi_end)
    q0 = min(perp(p) for p in pts)
    q1 = max(perp(p) for p in pts)
    at = lambda s, q: (a[0] + s * ux - q * uy, a[1] + s * uy + q * ux)
    return (at(s0, q0), at(s1, q0)), (at(s0, q1), at(s1, q1)), horizontal


def thicken(l1, l2, thickness, grow):
    """Widen a thin parapet to ``thickness`` world units towards native ``grow`` y.

    The authored face nearest the viewer (``grow='-y'``) or farthest from it
    (``grow='+y'``) stays fixed; the other face moves across the wall walk.
    """
    mid1 = G.lerp2(l1[0], l1[1], 0.5)
    mid2 = G.lerp2(l2[0], l2[1], 0.5)
    keep, move = (l1, l2) if (mid1[1] > mid2[1]) == (grow == '-y') else (l2, l1)
    dx, dy = move[0][0] - keep[0][0], move[0][1] - keep[0][1]
    # Perpendicular (world) unit normal of the run, pointing from keep to move.
    rx, ry = keep[1][0] - keep[0][0], (keep[1][1] - keep[0][1]) / G.S
    L = math.hypot(rx, ry)
    nx, ny = -ry / L, rx / L
    if nx * dx + ny * dy / G.S < 0:
        nx, ny = -nx, -ny
    off = (nx * thickness, ny * thickness * G.S)
    moved = ((keep[0][0] + off[0], keep[0][1] + off[1]), (keep[1][0] + off[0], keep[1][1] + off[1]))
    return keep, moved


def measured_parapet(data, spec):
    """Crenellated thin parapet from an authored quad and measured notch features.

    ``spec['features']`` are source pixel coordinates (x for horizontal runs,
    y for vertical ones) of the shadowed/lit merlon side faces; ``side`` says
    whether the notch lies before (``-1``) or after (``+1``) each feature along
    the run; ``notch_world`` is the notch length in world units.
    """
    l1, l2, horizontal = quad_lines(data)
    if spec.get('thickness'):
        l1, l2 = thicken(l1, l2, spec['thickness'], spec['grow'])
    zmid = (spec['floor'] + spec['top']) / 2
    mid0 = G.lerp2(l1[0], l2[0], 0.5)
    mid1 = G.lerp2(l1[1], l2[1], 0.5)
    if abs(mid1[0] - mid0[0]) > 10:
        coord0, coord1 = mid0[0], mid1[0]
    else:
        coord0, coord1 = mid0[1] - zmid, mid1[1] - zmid
    run = (mid1[0] - mid0[0], (mid1[1] - mid0[1]) / G.S)
    w = spec['notch_world'] / math.hypot(*run)
    notches = []
    for f in spec['features']:
        t = (f - coord0) / (coord1 - coord0)
        lo, hi = (t - w, t) if spec['side'] < 0 else (t, t + w)
        lo, hi = max(lo, 0.0), min(hi, 1.0)
        if hi - lo > 0.2 * w:
            notches.append((lo, hi))
    shell = G.crenellated_wall(l1, l2, spec['ground'], spec['floor'], spec['top'], notches)
    return shell, len(notches)


def trimmed(data, ground=PLATEAU, top=None, eps=1.0):
    """Authored pillar cut at the terrain datum; optional flat/sloped cap override."""
    if top is None:
        return G.native_top_prism(data, ground, eps)
    lifted = {'faces': data['faces'], 'verts': [
        (x, y, z if z < eps else (top(x, y) if callable(top) else top)) for x, y, z in data['verts']]}
    return G.native_top_prism(lifted, ground, eps)


def closed_as_is(data):
    """Weld and cap an authored floating volume without changing its shape."""
    return G.native_top_prism(data, -1e9, eps=-1e9)


# --------------------------------------------------------------------------
# Eastern curtain south run (140 walk, 142/143 outer parapets)
# --------------------------------------------------------------------------

SOUTH_RUN = {
    # Walk top 344: painted inner walk edge vs authored 140 edge (~350) balanced against the
    # mask 236 inner boundary (a 350 walk overshot it by ~5 px in the coverage audit).
    'walk': 344.0,
    142: {'features': [2531.1 + 8.84 * k for k in range(7)], 'side': -1,
          'notch_world': 10.0, 'floor': 364.0, 'top': 374.0, 'ground': PLATEAU,
          'thickness': 12.0, 'grow': '-y'},
    143: {'features': [2433.6, 2454.4, 2475.2, 2496.1, 2516.1], 'side': -1,
          'notch_world': 10.0, 'floor': 362.0, 'top': 374.0, 'ground': PLATEAU,
          'thickness': 12.0, 'grow': '-y'},
}


def build_south_run(base):
    out = {140: (trimmed(base[140], PLATEAU, SOUTH_RUN['walk']), 'Measured Wall Walk')}
    counts = {}
    for node in (142, 143):
        shell, counts[node] = measured_parapet(base[node], SOUTH_RUN[node])
        out[node] = (shell, 'Measured Crenellated Parapet')
    return out, {'walk_z': SOUTH_RUN['walk'], 'notches': counts, 'ground_z': PLATEAU}


# --------------------------------------------------------------------------
# Eastern curtain middle run (141 walk, 150 stair ramp, 157 face slope,
# 151/159/158 outer parapets)
# --------------------------------------------------------------------------

MIDDLE_RUN = {
    'walk': 344.0,
    151: {'features': [2649.9, 2668.5, 2687.2, 2705.9, 2724.3], 'side': -1,
          'notch_world': 10.0, 'floor': 366.0, 'top': 377.0, 'ground': PLATEAU, 'thickness': 12.0, 'grow': '-y'},
    159: {'features': [2743.7, 2761.9, 2780.2, 2799.4], 'side': -1,
          'notch_world': 10.0, 'floor': 366.0, 'top': 377.0, 'ground': PLATEAU, 'thickness': 12.0, 'grow': '-y'},
    158: {'features': [2818.3, 2837.0, 2857.0], 'side': -1,
          'notch_world': 10.0, 'floor': 367.0, 'top': 378.0, 'ground': PLATEAU, 'thickness': 12.0, 'grow': '-y'},
}


def build_middle_run(base):
    out = {141: (trimmed(base[141], PLATEAU, MIDDLE_RUN['walk']), 'Measured Wall Walk'),
           150: (trimmed(base[150], PLATEAU), 'Datum-Trimmed Stair Ramp'),
           157: (trimmed(base[157], PLATEAU), 'Datum-Trimmed Face Slope')}
    counts = {}
    for node in (151, 159, 158):
        shell, counts[node] = measured_parapet(base[node], MIDDLE_RUN[node])
        out[node] = (shell, 'Measured Crenellated Parapet')
    return out, {'walk_z': MIDDLE_RUN['walk'], 'notches': counts, 'ground_z': PLATEAU}


# --------------------------------------------------------------------------
# Eastern curtain corner turret (152)
# --------------------------------------------------------------------------

CORNER_TURRET = {'cx': 2893.0, 'cy': 1160.0, 'r_out': 32.0, 'r_in': 24.0,
                 'ground': PLATEAU, 'floor': 350.0, 'top': 361.0,
                 'merlons': {'count': 7, 'phase_deg': 180.0, 'width_deg': 30.0}}


def build_corner_turret(base):
    c = CORNER_TURRET
    m = c['merlons']
    merlons = G.merlon_intervals(m['phase_deg'], 360.0 / m['count'], m['width_deg'], m['count'])
    ring = G.crenellated_ring(c['cx'], c['cy'], c['r_out'], c['r_in'], c['ground'],
                              c['floor'], c['top'], merlons, step_deg=9.0)
    # Separate closed core (0.3 units inside the tube) forms the platform floor.
    G.cylinder(ring, c['cx'], c['cy'], c['r_in'] - 0.3, c['ground'], c['floor'] - 2.0, 40)
    return {152: (ring, 'Measured Round Turret')}, {
        'merlons': m['count'], 'ground_z': c['ground']}


# --------------------------------------------------------------------------
# North-eastern curtain (153/155 walks, 154/156 parapets)
# --------------------------------------------------------------------------

NE_CURTAIN = {
    'walk_north': 355.0, 'walk_south': 340.0, 'y_north': 999.0, 'y_south': 1125.0,
    # Painted merlon row (bright tops) on x 2865-2901; walkway paving x 2849-2865.
    'parapet_x': (2865.0, 2901.0), 'parapet_y': (992.0, 1143.0),
    'merlon_top_py': [657, 672, 686, 700, 713.5, 727.5, 742, 756],
    'merlon_len_y': 7.0, 'floor_rise': 12.0, 'top_rise': 24.0,
    156: {'t': [0.02, 0.15, 0.295, 0.43, 0.555, 0.695, 0.82, 0.955],
          'line': ((2801.7, 908.0), (2901.2, 996.2)), 'notch_world': 10.0,
          'floor': 367.0, 'top': 379.0},
}


def ne_walk_z(y):
    c = NE_CURTAIN
    t = min(max((y - c['y_north']) / (c['y_south'] - c['y_north']), 0.0), 1.0)
    return c['walk_north'] + (c['walk_south'] - c['walk_north']) * t


def build_ne_curtain(base):
    c = NE_CURTAIN
    x0, x1 = c['parapet_x']
    y0, y1 = c['parapet_y']
    walk = G.Shell()
    poly = [(2849.0, 999.0), (x0, 999.0), (x0, 1125.0), (2849.0, 1125.0)]
    walk.prism(poly, PLATEAU, [ne_walk_z(p[1]) for p in poly])
    # East parapet along y: merlon centres from bright top faces.
    centres = []
    for py in c['merlon_top_py']:
        # Solve y - (walk(y) + top_rise) = py for the merlon top.
        y = py + c['walk_north'] + c['top_rise']
        for _ in range(20):
            y = py + ne_walk_z(y) + c['top_rise']
        centres.append(y)
    cuts = {y0, y1}
    merl = []
    for yc in centres:
        a, b = yc - c['merlon_len_y'] / 2, yc + c['merlon_len_y'] / 2
        merl.append((a, b))
        cuts |= {a, b}
    cuts = sorted(v for v in cuts if y0 <= v <= y1)
    shell = G.Shell()
    for a, b in zip(cuts, cuts[1:]):
        mid = (a + b) / 2
        za, zb = ne_walk_z(a), ne_walk_z(b)
        fl = (za + c['floor_rise'],) * 2 + (zb + c['floor_rise'],) * 2
        tp = (za + c['top_rise'],) * 2 + (zb + c['top_rise'],) * 2
        shell.hexa((x0, a), (x1, a), (x0, b), (x1, b), PLATEAU, fl)
        if any(lo < mid < hi for lo, hi in merl):
            shell.hexa((x0, a), (x1, a), (x0, b), (x1, b), fl, tp)
    shell.cancel()
    out = {153: (walk, 'Measured Sloped Wall Walk'), 154: (shell, 'Measured Crenellated Parapet'),
           155: (trimmed(base[155], PLATEAU), 'Datum-Trimmed Wall Walk')}
    s = c[156]
    l1, l2, _ = quad_lines(base[156])
    run = (l1[1][0] - l1[0][0], (l1[1][1] - l1[0][1]) / G.S)
    w = s['notch_world'] / math.hypot(*run)
    # Features were measured along ``line``; map to the quad parameter by x.
    a, b = s['line']
    notches = []
    for t in s['t']:
        x = a[0] + (b[0] - a[0]) * t
        tt = (x - l1[0][0]) / (l1[1][0] - l1[0][0])
        lo, hi = max(tt, 0.0), min(tt + w, 1.0)
        if hi > lo:
            notches.append((lo, hi))
    out[156] = (G.crenellated_wall(l1, l2, PLATEAU, s['floor'], s['top'], notches),
                'Measured Crenellated Parapet')
    return out, {'east_merlons': len(centres), 'diagonal_notches': len(notches),
                 'walk_z': [c['walk_north'], c['walk_south']], 'ground_z': PLATEAU}


# --------------------------------------------------------------------------
# Round-2 revisions (user review): middle run, corner turret, north-eastern
# curtain.  Explicit coordinates only, so the builders work from the round-2
# baseline (which already holds the round-1 meshes).
# --------------------------------------------------------------------------

# Shared castle-wall levels, from the rectified middle-run parapet (notch
# shadow slits span z 370-380 along the authored 151/158 outer face) and the
# gate towers (walk +12 crenel floor, +24 merlon top).
WALK_Z, CRENEL_Z, MERLON_Z = 357.0, 369.0, 381.0
# Thickness direction of the middle run (perpendicular to the run, per world unit).
MID_U = (-0.652, -0.435)


def mid_front(x):
    """Outer face of the middle run: authored 151 and 158 outer edges are collinear."""
    return (x, 1302.0 - 0.4893 * (x - 2648.0))


def mid_offset(p, d):
    return (p[0] + MID_U[0] * d, p[1] + MID_U[1] * d)


MIDDLE_R2 = {
    # Shadowed west end faces of merlons (source px x) along the outer face line;
    # the notch lies west of each face.  2780.5 is hidden by ivy (pitch-inferred).
    'slits': [2650.0, 2668.5, 2687.0, 2706.0, 2724.5, 2743.0, 2762.0, 2780.5, 2799.5,
              2818.5, 2837.0, 2857.0],
    'notch_x': 9.1,                       # 12 world units along the run
    'parapet_d': 8.0,                     # parapet thickness (world)
    'x_start': 2641.0, 'x_end': 2872.0,   # slate-tower corner turret -> corner turret
    'splits': {151: (2641.0, 2729.0), 159: (2729.0, 2808.0), 158: (2808.0, 2872.0)},
    # Walk inner edge from the mask 239 upper boundary at walk height 357.
    'inner': lambda x: 1268.0 - 0.5 * (x - 2650.0),
    'walk_x': (2628.0, 2866.0),
    # Stair 150 along the inner face: courtyard (z 220, west) to the walk (east).
    'stair': [(2692.1, 1201.6), (2716.0, 1235.0), (2816.2, 1185.0), (2790.0, 1153.1)],
}


def _crenel_run(x0, x1, notches_x, d0, d1, z_base):
    a, b = mid_front(x0), mid_front(x1)
    outer = (mid_offset(a, d0), mid_offset(b, d0))
    inner = (mid_offset(a, d1), mid_offset(b, d1))
    notches = []
    for lo, hi in notches_x:
        t0, t1 = (lo - x0) / (x1 - x0), (hi - x0) / (x1 - x0)
        t0, t1 = max(t0, 0.0), min(t1, 1.0)
        if t1 - t0 > 1e-3:
            notches.append((t0, t1))
    return G.crenellated_wall(outer, inner, z_base, CRENEL_Z, MERLON_Z, notches), len(notches)


def build_middle_run_r2(base):
    c = MIDDLE_R2
    notches_x = [(x - c['notch_x'], x) for x in c['slits']]
    out, counts = {}, {}
    for node, (x0, x1) in c['splits'].items():
        shell, counts[node] = _crenel_run(x0, x1, notches_x, 0.0, c['parapet_d'], PLATEAU)
        out[node] = (shell, 'Measured Crenellated Parapet')
    # Walk 141: between the parapet back and the painted inner edge, level at 357.
    wx0, wx1 = c['walk_x']
    back0, back1 = mid_offset(mid_front(wx0), c['parapet_d']), mid_offset(mid_front(wx1), c['parapet_d'])
    walk = G.Shell()
    walk.prism([back0, back1, (back1[0], c['inner'](back1[0])), (back0[0], c['inner'](back0[0]))],
               PLATEAU, WALK_Z)
    out[141] = (walk, 'Level Wall Walk')
    # Stair 150: stepped flight along the inner face, courtyard (west) up to the
    # walk (east); painted treads show above the parapet at x 2760-2800.
    a, b, cc, d = c['stair']
    n = 18
    rise = (WALK_Z - PLATEAU) / n
    prof = [(0.0, PLATEAU)]
    for k in range(n):
        prof += [(k / n, PLATEAU + rise * (k + 1)), ((k + 1) / n, PLATEAU + rise * (k + 1))]
    prof += [(1.0, PLATEAU)]
    u = (d[0] - a[0], d[1] - a[1])
    wv = (b[0] - a[0], b[1] - a[1])
    out[150] = (G.extrude_profile(prof, a, u, wv, (0.0, 1.0)), 'Stepped Stair To Walk')
    # 157: the authored full-height outward slope contradicted the straight
    # painted face (the source parapet is on one line).  Kept as an internal
    # base course flush behind the outer face so it adds no visible surface.
    p0, p1 = mid_front(2728.0), mid_front(2821.0)
    course = G.Shell()
    course.prism([mid_offset(p0, 1.0), mid_offset(p1, 1.0), mid_offset(p1, 7.0), mid_offset(p0, 7.0)],
                 PLATEAU, PLATEAU + 30.0)
    out[157] = (course, 'Internal Base Course')
    return out, {'walk_z': WALK_Z, 'crenel_z': CRENEL_Z, 'merlon_z': MERLON_Z,
                 'notches': counts, 'ground_z': PLATEAU}


CORNER_R2 = {'cx': 2886.0, 'cy': 1176.0, 'r': 35.0, 'r_in': 29.0,
             # Painted outline tapers from width 66 (px 840-847 at the sides) to a
             # 24-wide shaft (px ~900) that continues down to the ground.
             'corbel': (285.0, 12.0, 329.0),  # z_bottom, r_bottom, z_top
             'shaft': (12.0, 220.0, 284.9),
             'merlons': {'count': 8, 'phase_deg': 10.0, 'width_deg': 22.5}}


def frustum(shell, cx, cy, r0, z0, r1, z1, segments=40):
    lo = [(*G.ellipse_point(cx, cy, r0, 2 * math.pi * i / segments), z0) for i in range(segments)]
    hi = [(*G.ellipse_point(cx, cy, r1, 2 * math.pi * i / segments), z1) for i in range(segments)]
    shell.face(list(reversed(lo)))
    shell.face(hi)
    for i in range(segments):
        j = (i + 1) % segments
        shell.face([lo[i], lo[j], hi[j], hi[i]])
    return shell


def build_corner_turret_r2(base):
    c = CORNER_R2
    m = c['merlons']
    merlons = G.merlon_intervals(m['phase_deg'], 360.0 / m['count'], m['width_deg'], m['count'])
    zb, rb, zt = c['corbel']
    shell = G.crenellated_ring(c['cx'], c['cy'], c['r'], c['r_in'], zt, CRENEL_Z, MERLON_Z,
                               merlons, step_deg=7.5)
    frustum(shell, c['cx'], c['cy'], rb, zb, c['r'], zt - 0.1)
    G.cylinder(shell, c['cx'], c['cy'], c['r_in'] - 0.2, zt, WALK_Z, 40)
    sr, sz0, sz1 = c['shaft']
    G.cylinder(shell, c['cx'], c['cy'], sr, sz0, sz1, 24)
    return {152: (shell, 'Corbelled Turret On Shaft')}, {'merlons': m['count'], 'corbel': c['corbel'],
                                                          'shaft': c['shaft'], 'ground_z': PLATEAU}


NE_R2 = {
    # East run: walk paving x 2848-2893, thin parapet strip x 2893-2901.  Merlon
    # tops are the bright 7-8 px runs along x 2895-2897 (px y + 381 = native y).
    'walk': ((2848.0, 2893.0), (999.0, 1172.0)),  # meets walk 155 at y 999
    'parapet_x': (2893.0, 2901.0), 'parapet_y': (992.0, 1165.0),
    'merlons_y': [(995, 1002), (1009, 1017), (1024, 1032), (1038, 1046), (1052, 1059),
                  (1067, 1074), (1081, 1089), (1096, 1104), (1110, 1117), (1124, 1132),
                  (1139, 1146), (1153, 1160)],
    # Diagonal run: merlon caps (bright at z 380) / shadowed faces (dark at z 372)
    # measured along the authored line (2801.7, 908) -> (2901.2, 996.2).
    'diag_line': ((2801.7, 908.0), (2901.2, 996.2)),
    'diag_quad': {'outer': ((2805.1, 906.55), (2905.0, 994.8)),
                  'inner': ((2798.2, 909.1), (2898.1, 997.35))},
    'diag_merlons_t': [(0.065 + 0.1357 * k, 0.065 + 0.1357 * k + 0.0704) for k in range(7)] + [(0.95, 1.0)],
    'walk155': [(2778.9, 936.6), (2781.6, 920.2), (2800.4, 910.4), (2898.1, 997.35), (2893.0, 999.0),
                (2848.0, 999.0)],
}


def build_ne_curtain_r2(base):
    c = NE_R2
    (wx0, wx1), (wy0, wy1) = c['walk']
    walk = G.Shell()
    walk.prism([(wx0, wy0), (wx1, wy0), (wx1, wy1), (wx0, wy1)], PLATEAU, WALK_Z)
    px0, px1 = c['parapet_x']
    py0, py1 = c['parapet_y']
    cuts = sorted({py0, py1} | {v for m in c['merlons_y'] for v in m if py0 < v < py1})
    par = G.Shell()
    for a, b in zip(cuts, cuts[1:]):
        par.hexa((px0, a), (px1, a), (px0, b), (px1, b), PLATEAU, CRENEL_Z)
        if any(lo <= (a + b) / 2 <= hi for lo, hi in c['merlons_y']):
            par.hexa((px0, a), (px1, a), (px0, b), (px1, b), CRENEL_Z, MERLON_Z)
    par.cancel()
    (ax, ay), (bx, by) = c['diag_line']
    q = c['diag_quad']
    ox0, ox1 = q['outer'][0][0], q['outer'][1][0]
    # Map merlon parameters (measured along diag_line) to the quad by x.
    def tq(t):
        return ((ax + (bx - ax) * t) - ox0) / (ox1 - ox0)
    merl = [(max(tq(a), 0.0), min(tq(b), 1.0)) for a, b in c['diag_merlons_t']]
    edges = sorted({0.0, 1.0} | {v for m in merl for v in m if 0 < v < 1})
    notches = [(a, b) for a, b in zip(edges, edges[1:]) if not any(lo <= (a + b) / 2 <= hi for lo, hi in merl)]
    diag = G.crenellated_wall(q['outer'], q['inner'], PLATEAU, CRENEL_Z, MERLON_Z, notches)
    walk155 = G.Shell()
    walk155.prism(c['walk155'], PLATEAU, WALK_Z)
    return {153: (walk, 'Level Wall Walk'), 154: (par, 'Measured Crenellated Parapet'),
            155: (walk155, 'Level Wall Walk'), 156: (diag, 'Measured Crenellated Parapet')}, {
        'east_merlons': len(c['merlons_y']), 'diagonal_merlons': len(merl), 'walk_z': WALK_Z,
        'ground_z': PLATEAU}


# --------------------------------------------------------------------------
# North-eastern square tower (163 body, 160 parapet, 161 front band,
# 162 turret, 164/165 spire halves)
# --------------------------------------------------------------------------

NE_TOWER = {
    # Paired inner/outer stations of the authored parapet 160 (open polyline).
    'inner': [(2853.0, 841.2), (2676.0, 799.0), (2638.0, 762.0), (2566.0, 771.0),
              (2556.0, 809.0), (2538.0, 829.0), (2675.0, 868.0), (2631.0, 928.0),
              (2649.0, 933.0), (2638.0, 947.0), (2717.0, 966.0), (2728.0, 953.0),
              (2759.0, 959.0), (2830.7, 864.9)],
    'outer': [(2856.1, 837.3), (2680.0, 794.0), (2641.3, 757.8), (2561.3, 766.3),
              (2547.0, 806.0), (2530.0, 833.0), (2665.0, 870.0), (2620.0, 929.0),
              (2640.0, 936.0), (2629.3, 948.3), (2721.0, 971.0), (2730.4, 958.5),
              (2762.0, 965.5), (2836.7, 865.2)],
    'base': 407.0, 'floor': 426.0, 'top': 438.0,
    'period_world': 27.0, 'merlon_world': 15.0,
    'turret': {'cx': 2834.7, 'cy': 831.3, 'r': 37.0, 'z0': 363.0, 'z1': 466.0,
               'spire_r': 40.0, 'apex': 542.0},
}


def corner_anchored_notches(length, period, merlon):
    """Notch intervals (fractions) with merlons at both corners of a segment."""
    n = max(int(round(length / period)), 1)
    if length < merlon * 1.6:
        return []
    pitch = (length - merlon) / n
    notch = pitch - merlon
    if notch <= 2.0:
        n = max(n - 1, 1)
        pitch = (length - merlon) / n
        notch = pitch - merlon
    return [((merlon + k * pitch) / length, (merlon + k * pitch + notch) / length) for k in range(n)]


def build_ne_tower(base):
    c = NE_TOWER
    stations = list(zip(c['outer'], c['inner']))
    shell = G.Shell()
    total_notches = 0
    for i in range(len(stations) - 1):
        (ao, ai), (bo, bi) = stations[i], stations[i + 1]
        mo, mb = G.lerp2(ao, ai, 0.5), G.lerp2(bo, bi, 0.5)
        length = math.hypot(mb[0] - mo[0], (mb[1] - mo[1]) / G.S)
        notches = corner_anchored_notches(length, c['period_world'], c['merlon_world'])
        total_notches += len(notches)
        cuts = sorted({0.0, 1.0} | {t for n in notches for t in n})
        for t0, t1 in zip(cuts, cuts[1:]):
            p0o, p0i = G.lerp2(ao, bo, t0), G.lerp2(ai, bi, t0)
            p1o, p1i = G.lerp2(ao, bo, t1), G.lerp2(ai, bi, t1)
            shell.hexa(p0o, p0i, p1o, p1i, c['base'], c['floor'])
            if not any(lo < (t0 + t1) / 2 < hi for lo, hi in notches):
                shell.hexa(p0o, p0i, p1o, p1i, c['floor'], c['top'])
    shell.cancel()
    t = c['turret']
    turret = G.cylinder(G.Shell(), t['cx'], t['cy'], t['r'], t['z0'], t['z1'], 40).cancel()
    out = {163: (trimmed(base[163], PLATEAU), 'Datum-Trimmed Tower Body'),
           160: (shell, 'Measured Crenellated Parapet'),
           161: (closed_as_is(base[161]), 'Closed Front Parapet Band'),
           162: (turret, 'Measured Round Turret')}
    for node, (a0, a1) in ((164, (math.pi / 2, 3 * math.pi / 2)), (165, (-math.pi / 2, math.pi / 2))):
        cone = G.Shell()
        seg = 12
        ring = [G.ellipse_point(t['cx'], t['cy'], t['spire_r'], a0 + (a1 - a0) * i / seg) for i in range(seg + 1)]
        apex = (t['cx'], t['cy'], t['apex'])
        centre = (t['cx'], t['cy'], t['z1'])
        base_pts = [(x, y, t['z1']) for x, y in ring]
        cone.face([centre] + list(reversed(base_pts)))
        for i in range(seg):
            cone.face([base_pts[i], base_pts[i + 1], apex])
        cone.face([base_pts[0], centre, base_pts[-1], apex])
        out[node] = (cone, 'Measured Half Spire')
    return out, {'parapet_segments': len(stations) - 1, 'parapet_notches': total_notches,
                 'ground_z': PLATEAU}


# --------------------------------------------------------------------------
# North-eastern tower stair (167 flight, 166 landing, 71 strip)
# --------------------------------------------------------------------------

NE_STAIR = {'steps': 12, 'bottom': ((2628.4, 951.4), (2642.2, 931.7)),
            'top': ((2692.1, 966.0), (2705.9, 946.3)), 'z_top': 265.0, 'thickness': 6.0}


def build_ne_stair(base):
    c = NE_STAIR
    n = c['steps']
    rise = (c['z_top'] - PLATEAU) / n
    prof = [(0.0, PLATEAU)]
    for k in range(n):
        z = PLATEAU + rise * (k + 1)
        prof += [(k / n, z), ((k + 1) / n, z)]
    t_soffit = c['thickness'] / (c['z_top'] - PLATEAU)
    prof += [(1.0, c['z_top'] - c['thickness']), (t_soffit, PLATEAU)]
    (b0, b1), (t0, _) = c['bottom'], c['top']
    origin = b0
    u = (t0[0] - b0[0], t0[1] - b0[1])
    w = (b1[0] - b0[0], b1[1] - b0[1])
    stair = G.extrude_profile(prof, origin, u, w, (0.0, 1.0))
    strip = G.Shell()
    pts = footprint_corners(base[71])
    # Hull order around the centroid for the flat marker slab.
    cx = sum(p[0] for p in pts) / len(pts)
    cy = sum(p[1] for p in pts) / len(pts)
    pts.sort(key=lambda p: math.atan2(p[1] - cy, p[0] - cx))
    hull = []
    for p in pts:
        hull.append(p)
    strip.prism(hull, PLATEAU, PLATEAU + 1.0)
    return {167: (stair, 'Measured Stepped Flight'), 166: (closed_as_is(base[166]), 'Closed Landing'),
            71: (strip, 'Ground-Level Marker Strip')}, {'steps': n, 'ground_z': PLATEAU}


# --------------------------------------------------------------------------
# Eastern slate tower (144 corner turret, 145/146/147 walls, 148 eave, 149 roof)
# --------------------------------------------------------------------------

SLATE = {'turret_ground': 205.0, 'eave_z': 397.6,
         'ridge': ((2515.4, 1273.4, 465.0), (2599.4, 1301.4, 460.8))}


def hip_roof(eave, z, ridge):
    a, b = ridge
    ax = (b[0] - a[0], (b[1] - a[1]) / G.S)
    shell = G.Shell()
    def along(p):
        return (p[0] - a[0]) * ax[0] + (p[1] - a[1]) / G.S * ax[1]
    mid = along(b) / 2
    owner = [a if along(p) < mid else b for p in eave]
    pts = [(p[0], p[1], z) for p in eave]
    shell.face(list(reversed(pts)))
    n = len(pts)
    for i in range(n):
        j = (i + 1) % n
        if owner[i] is owner[j]:
            shell.face([pts[i], pts[j], owner[i]])
        else:
            shell.face([pts[i], pts[j], owner[j], owner[i]])
    return shell


def build_slate_tower(base):
    eave = footprint_corners(base[148], zmax=396.0)
    cx = sum(p[0] for p in eave) / len(eave)
    cy = sum(p[1] for p in eave) / len(eave)
    eave.sort(key=lambda p: math.atan2(p[1] - cy, p[0] - cx))
    out = {144: (trimmed(base[144], SLATE['turret_ground']), 'Datum-Trimmed Corner Turret'),
           145: (trimmed(base[145]), 'Datum-Trimmed Wall'),
           146: (trimmed(base[146]), 'Datum-Trimmed Wall'),
           147: (trimmed(base[147]), 'Datum-Trimmed Wall'),
           148: (closed_as_is(base[148]), 'Closed Eave Slab'),
           149: (hip_roof(eave, SLATE['eave_z'], SLATE['ridge']), 'Closed Hip Roof')}
    return out, {'ground_z': PLATEAU, 'turret_ground_z': SLATE['turret_ground'],
                 'roof_eave_vertices': len(eave)}


# --------------------------------------------------------------------------
# Inner gatehouse (77, 93, 123, 130-138): coped, not crenellated
# --------------------------------------------------------------------------

INNER_NODES = [77, 93, 123, 130, 131, 132, 133, 134, 135, 136, 137, 138]


# Rear curtains 132/93 and the rear lintel 133 are authored as 4-unit skins,
# but their painted coping spans 1030-1048 px: the wall is ~16 native y
# (28 world units) deep.  The front faces stay; the back faces move north.
INNER_REAR = {132: (PLATEAU, 363.0), 93: (PLATEAU, 363.0), 133: (276.0, 363.0)}
INNER_REAR_DEPTH = 16.0


# West half-round tower 130 (round 2).  The authored skin put the west flank
# at (1989, 1480), 13 native units in front of the circle through its painted
# front (2015/1488, 2039/1486) and flanks (x 1989 / 2062).  A half-round of
# world radius 36.5 about (2025.5, 1467.1) fits the painted round silhouette and
# joins the curtain strip at y ~1465-1467; the old flank intersected the
# courtyard privy 196 and hid its painted roof verge (x 1976-1999, px 1175-1213).
INNER_WEST_TOWER = {'cx': 2025.5, 'cy': 1467.1, 'r_out': 36.5, 'r_in': 31.5,
                    'east_deg': 16.0, 'z0': PLATEAU, 'z1': 365.0}


def inner_west_tower():
    t = INNER_WEST_TOWER
    arc = lambda r, a0, a1, n: [G.ellipse_point(t['cx'], t['cy'], r, math.radians(a0 + (a1 - a0) * i / n))
                                for i in range(n + 1)]
    outer = [(1956.6, 1461.5), (1979.9, 1465.0)] + arc(t['r_out'], 180.0, t['east_deg'], 20) + [(2094.0, 1476.0)]
    inner = [(2096.0, 1471.9)] + arc(t['r_in'], t['east_deg'] - 4.0, 180.0, 20) + [(1988.0, 1459.9), (1957.8, 1457.4)]
    shell = G.Shell()
    shell.prism(outer + inner, t['z0'], t['z1'])
    return shell


def build_inner_gatehouse(base):
    out = {}
    for node, (z0, z1) in INNER_REAR.items():
        l1, l2, _ = quad_lines(base[node])
        front, back = (l1, l2) if (l1[0][1] + l1[1][1]) > (l2[0][1] + l2[1][1]) else (l2, l1)
        back = tuple((p[0], f[1] - INNER_REAR_DEPTH) for p, f in zip(back, front))
        shell = G.Shell()
        shell.prism([front[0], front[1], back[1], back[0]], z0, z1)
        out[node] = (shell, 'Thickened Coped Wall')
    out[130] = (inner_west_tower(), 'Half-Round West Tower')
    for node in INNER_NODES:
        if node in out:
            continue
        zmin = min(v[2] for v in base[node]['verts'])
        if zmin < 1.0:
            out[node] = (trimmed(base[node]), 'Datum-Trimmed Part')
        else:
            out[node] = (closed_as_is(base[node]), 'Closed Floating Part')
    return out, {'ground_z': PLATEAU, 'crenellations': 'none (continuous coping in source)'}


def build_ne_stair_r2(base):
    """Round 2: marker strip 71 no longer drawn as a long ground slab.

    71 is a non-visual sight obstacle whose authored top is exactly the plateau
    (native z 220) and which runs from the stair foot north across the tower
    footprint; it has no painted counterpart.  Until the coordinator moves it
    (grouping-proposal.json: whole node to the north-bailey plateau terrain),
    it is kept as a small closed block inside the stair's first step so the
    stair asset shows no stray piece.  Flight 167 and landing 166 keep their
    round-1 geometry (rebuilt identically).
    """
    out, facts = build_ne_stair(base)
    marker = G.Shell()
    marker.prism([(2634.0, 946.0), (2640.5, 936.5), (2645.0, 944.0)], PLATEAU - 1.0, PLATEAU + 2.0)
    out[71] = (marker, 'Hidden Marker Stub')
    return out, {**facts, 'marker_71': 'stub inside first step'}


# --------------------------------------------------------------------------
# Round 3 (catalog v3 regroup)
# --------------------------------------------------------------------------

LOWER_WALK_R3 = {
    # Walk component 083 "east-curtain-walk" (authored by the south lane at z 350)
    # is raised to the lane walk height 357.  Its inner edge (36-37 native units
    # from the parapet 098 inner line) moves 7 units towards the viewer so the
    # painted inner paving edge (pixel = y - z) stays where the z-350 walk fitted it.
    'line': ((2489.8, 1665.6), (2377.3, 1811.7)), 'inner_sd': 30.0, 'rise': 7.0,
    'from_z': 350.0, 'to_z': WALK_Z,
}


def build_lower_r3(base):
    c = LOWER_WALK_R3
    data = base[83]
    (ax, ay), (bx, by) = c['line']
    L = math.hypot(bx - ax, by - ay)
    verts = []
    moved = 0
    for x, y, z in data['verts']:
        sd = ((bx - ax) * (y - ay) - (by - ay) * (x - ax)) / L
        if sd > c['inner_sd']:
            y += c['rise']
            moved += 1
        if abs(z - c['from_z']) < 0.05:
            z = c['to_z']
        verts.append((x, y, z))
    shell = G.Shell(digits=4)
    for f in data['faces']:
        shell.face([verts[i] for i in f])
    return {83: (shell, 'Walk Raised To Lane Height')}, {
        'walk_z': c['to_z'], 'inner_edge_vertices_moved': moved, 'ground_z': 'unchanged (ravine foot 177-204)'}


SLATE_FLOOR_R3 = {
    # Tower footprint: outer lines of walls 146 (west), 147 (north), 145 (south)
    # and corner turret 144 (south-east); the east side has no wall obstacle.
    'poly': [(2502.0, 1300.0), (2505.6, 1295.0), (2540.0, 1259.4), (2545.0, 1261.2), (2600.6, 1277.8),
             (2641.3, 1293.0), (2646.0, 1317.8), (2626.0, 1333.0), (2604.0, 1334.0), (2586.0, 1323.8),
             (2562.0, 1317.0)],
    'top': 344.0,   # level of the south walk that enters the tower
}


def build_slate_floor_r3(base):
    c = SLATE_FLOOR_R3
    shell = G.Shell()
    shell.prism(c['poly'], PLATEAU, c['top'])
    return {140: (shell, 'Tower Floor Block')}, {'floor_z': c['top'], 'ground_z': PLATEAU}


# Coordinator decision (round 3): walk 083 "east-curtain-walk" stays at z 350 so it
# meets the south lane's corner turret and curtain 084 walks without a step;
# build_lower_r3 is kept for reference only and is not registered.
BUILDERS_R3 = {
    'lincoln-east-slate-tower': build_slate_floor_r3,
}

BUILDERS = {
    'lincoln-east-curtain-wall-south': build_south_run,
    'lincoln-east-curtain-wall-middle': build_middle_run_r2,
    'lincoln-east-corner-turret': build_corner_turret_r2,
    'lincoln-northeast-curtain-wall': build_ne_curtain_r2,
    'lincoln-northeast-square-tower': build_ne_tower,
    'lincoln-northeast-tower-stair': build_ne_stair_r2,
    'lincoln-east-slate-tower': build_slate_tower,
    'lincoln-inner-gatehouse': build_inner_gatehouse,
    'lincoln-east-gate-north-tower': lambda base: build_round_tower(ROUND_TOWERS['lincoln-east-gate-north-tower'], base),
    'lincoln-east-gate-south-tower': lambda base: build_round_tower(ROUND_TOWERS['lincoln-east-gate-south-tower'], base),
    'lincoln-east-gate-arch': build_gate_arch,
    'lincoln-east-curtain-wall-lower': build_lower_curtain,
}


# Working receiver-mask revisions (owned nodes only).  The frozen review gave
# the walk slab 79, south pier 82 and outer parapet 96 the south-tower
# silhouette 233 alone, although their painted walk paving, pier face above
# the arch and parapet merlons lie inside the gate-wall silhouette 234
# (evidence: inspection/mask-revision-234.png).  Both masks stay subject to
# first-hit receiver gating.
MASK_REVISIONS = {
    # User: "stairs missing texture?" - the painted treads of stair 150 lie in
    # silhouette 240 (walk/corner-turret envelope), not in 239/409
    # (inspection/mask-revision-150-240.png; 744 of 1374 stair first-hit px).
    'lincoln-east-curtain-wall-middle': {'building-150': [239, 240, 409]},
    'lincoln-east-gate-arch': {
        'building-079': [233, 234], 'building-082': [233, 234], 'building-096': [233, 234],
    },
}


# Reviewed foreground-vegetation exclusions for owned receivers (PROCEDURE
# section 4).  Native foliage masks that are painted *in front of* the owned
# masonry; evidence sheets inspection/vegetation-exclusions.png.  Tree masks
# that the frozen review judged to lie behind the architecture (53/54/156) are
# deliberately not excluded.
VEGETATION = {
    'lincoln-northeast-square-tower': ({'building-160', 'building-161', 'building-163'}, [157, 161, 82, 70],
        'Courtyard tree (native foliage masks 157/161), bush 82 and bush 70 stand in front of the west '
        'recess and front foot of the tower.'),
    'lincoln-northeast-curtain-wall': ({'building-153', 'building-154', 'building-155', 'building-156'}, [70],
        'Courtyard bush (mask 70) in front of the curtain walk foot near the stair.'),
    'lincoln-east-curtain-wall-middle': ({'building-141', 'building-150', 'building-151', 'building-157',
                                          'building-158', 'building-159'}, [71, 76, 155],
        'Courtyard bush 71 in front of the walk edge and bushes 76/155 in front of the outer wall foot.'),
    'lincoln-east-corner-turret': ({'building-152'}, [76, 155],
        'Bushes 76/155 in front of the turret foot.'),
    'lincoln-east-gate-south-tower': ({'building-080', 'building-097'}, [260],
        'Shrub on the rock slope (mask 260) in front of the tower foot.'),
    'lincoln-east-curtain-wall-lower': ({'building-098'}, [260],
        'Shrub on the rock slope (mask 260) in front of the wall foot.'),
}


def apply_mask_revisions(workspace, asset):
    revisions = MASK_REVISIONS.get(asset, {})
    vegetation = VEGETATION.get(asset)
    if not revisions and not vegetation:
        return []
    path = workspace / 'source-masks.json'
    data = json.loads(path.read_text())
    changed = []
    for row in data['projections']['exterior']['assignments']:
        node = row['source_node']
        if node in revisions and row['mask_indices'] != revisions[node]:
            row['mask_indices'] = list(revisions[node])
            row['requires_first_hit_gating'] = True
            row['worker_revision'] = ('east_gate_walls lane: painted pixels of this receiver lie in the added '
                                      'native silhouette; see inspection/mask-revision-*.png')
            changed.append(node)
        if vegetation and node in vegetation[0]:
            excl = sorted(set(row.get('exclude_mask_indices', [])) | set(vegetation[1]))
            if excl != row.get('exclude_mask_indices'):
                row['exclude_mask_indices'] = excl
                row['exclusions_reviewed'] = True
                prior = row.get('exclusion_reason', '')
                reason = 'Foreground vegetation (east_gate_walls review): ' + vegetation[2] + \
                         ' Evidence: inspection/vegetation-exclusions.png.'
                row['exclusion_reason'] = (prior + ' ' + reason).strip() if prior and reason not in prior else reason
                changed.append(node + ' vegetation')
    if changed:
        path.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
    return changed


# --------------------------------------------------------------------------
# Blender driver
# --------------------------------------------------------------------------

def _sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


# Round-2 component split (user: "again has a hut thing that should be
# separate" / slate tower: "this is what the above should be part of").  Walk
# 140 is cut by the vertical plane through the slate-tower south face (145
# outer edge, extended); the part inside the tower becomes its own component.
SPLIT_140 = {'p1': (2501.8, 1300.1), 'p2': (2562.0, 1316.9),
             'walk': 'east-curtain-south-walk', 'tower': 'slate-tower-floor-block'}


def split_south_walk(workspace, owned, report):
    """Replace mesh 140 by two closed components cut from the baseline mesh."""
    import bmesh
    import bpy
    from mathutils import Vector
    src = owned[140]
    matrix = src.matrix_world.copy()
    with bpy.data.libraries.load(str(workspace / 'baseline.blend'), link=False) as (lib_src, lib_dst):
        if src.name not in lib_src.objects:
            raise ValueError('Baseline lacks ' + src.name)
        lib_dst.objects = [src.name]
    base_obj = lib_dst.objects[0]
    base_mesh = base_obj.data
    bpy.data.objects.remove(base_obj)
    collection = bpy.data.collections[json.loads((workspace / 'workspace.json').read_text())['collection_name']]
    for obj in list(collection.all_objects):
        if obj.type == 'MESH' and obj.get('source_node') == 'building-140' and obj is not src:
            bpy.data.objects.remove(obj)   # idempotent: drop an earlier second component
    (x1, y1), (x2, y2) = SPLIT_140['p1'], SPLIT_140['p2']
    p1w = Vector(G.to_world((x1, y1, 0.0)))
    p2w = Vector(G.to_world((x2, y2, 0.0)))
    d = p2w - p1w
    normal = Vector((-d.y, d.x, 0.0)).normalized()   # points into the tower (north)
    test = Vector(G.to_world((2570.0, 1290.0, 300.0))) - p1w
    if normal.dot(test) < 0:
        normal = -normal
    inverse = matrix.inverted()
    results = []
    for keep_tower, component, label in ((False, SPLIT_140['walk'], 'Wall Walk Component'),
                                         (True, SPLIT_140['tower'], 'Slate Tower Floor Component')):
        bm = bmesh.new()
        bm.from_mesh(base_mesh)
        bm.transform(matrix)   # baseline mesh is local to the same (unchanged) transform
        # Concave n-gons (walk top/bottom) must be triangulated before bisecting.
        bmesh.ops.triangulate(bm, faces=bm.faces[:], quad_method='BEAUTY', ngon_method='EAR_CLIP')
        geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
        cut = bmesh.ops.bisect_plane(bm, geom=geom, plane_co=p1w, plane_no=normal,
                                     clear_inner=keep_tower, clear_outer=not keep_tower)
        edges = [e for e in cut['geom_cut'] if isinstance(e, bmesh.types.BMEdge)]
        bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=0)
        bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 4],
                              quad_method='BEAUTY', ngon_method='EAR_CLIP')
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bm.transform(inverse)
        if keep_tower:
            obj = src.copy()
            obj.data = src.data.copy()
            for coll in src.users_collection:
                coll.objects.link(obj)
            obj.name = src.name.replace('Roof or surface projection component 140',
                                        'Slate tower floor component 140')
        else:
            obj = src
        mesh = bpy.data.meshes.new(obj.name + ' ' + label)
        bm.to_mesh(mesh)
        nonmanifold = sum(not e.is_manifold for e in bm.edges)
        bm.free()
        for material in src.data.materials:
            mesh.materials.append(material)
        mesh.uv_layers.new(name='UVMap')
        old = obj.data
        obj.data = mesh
        if old.users == 0:
            bpy.data.meshes.remove(old)
        obj['source_node'] = 'building-140'
        obj['asset_group'] = src['asset_group']
        obj['projection_component'] = component
        obj['source_projection_current'] = False
        if [list(r) for r in obj.matrix_world] != [list(r) for r in matrix]:
            raise ValueError('Component transform drifted')
        results.append({'object': obj.name, 'projection_component': component, 'vertices': len(mesh.vertices),
                        'faces': len(mesh.polygons), 'nonmanifold_edges': nonmanifold,
                        'world_volume': round(G.mesh_volume(obj), 2)})
    tmp = bmesh.new()
    tmp.from_mesh(base_mesh)
    tmp.transform(matrix)
    bmesh.ops.triangulate(tmp, faces=tmp.faces[:], quad_method='BEAUTY', ngon_method='EAR_CLIP')
    base_volume = tmp.calc_volume(signed=True)
    tmp.free()
    if base_mesh.users == 0:
        bpy.data.meshes.remove(base_mesh)
    report['split_140'] = {'components': results, 'baseline_volume': round(base_volume, 2),
                           'component_volume_sum': round(sum(r['world_volume'] for r in results), 2)}
    return results


def run(asset, packet, packet_only=False, only_nodes=None, split_140=False, round3=False):
    import bpy
    sys.path.insert(0, str(HERE))
    from render_slots import acquire
    acquire()
    blend = Path(bpy.data.filepath).resolve()
    if blend.name != 'model.blend' or 'baseline' in blend.name:
        raise ValueError('Recipe only edits a workspace model.blend')
    workspace = blend.parent
    config = json.loads((workspace / 'workspace.json').read_text())
    if config['asset_id'] != asset:
        raise ValueError(f'Workspace belongs to {config["asset_id"]}, not {asset}')
    collection = bpy.data.collections[config['collection_name']]
    owned = {}
    for obj in collection.all_objects:
        if obj.type == 'MESH' and obj.get('asset_group') == asset:
            node = int(obj['source_node'].split('-')[1])
            if split_140 and obj.get('projection_component') == SPLIT_140['tower']:
                continue  # rebuilt from the baseline by split_south_walk
            if node in owned:
                raise ValueError(f'Duplicate owned mesh for node {node}')
            owned[node] = obj
    if packet_only:
        # Round 2+: baseline.blend already holds the reviewed geometry; keep it.
        generated, facts = {}, {'packet_only': True}
    else:
        base = G.load_baseline_native(workspace / 'baseline.blend', {o.name: o.matrix_world.copy() for o in owned.values()})
        base = {node: base[obj.name] for node, obj in owned.items()}
        generated, facts = (BUILDERS_R3 if round3 else BUILDERS)[asset](base)
        if only_nodes:
            # Round 2+: rebuild just these nodes; the rest keep the reviewed geometry.
            generated = {k: v for k, v in generated.items() if k in only_nodes}
            facts = {**facts, 'only_nodes': sorted(only_nodes)}
    changes = []
    for node, (shell, label) in sorted(generated.items()):
        if node not in owned:
            raise ValueError(f'Builder produced unowned node {node}')
        changes.append(G.replace_mesh(owned[node], shell, label))
    bad = [c for c in changes if c['nonmanifold_edges'] or c['degenerate_faces'] or c['world_volume'] <= 0]
    split_report = {}
    if split_140:
        if asset != 'lincoln-east-curtain-wall-south':
            raise ValueError('--split-140 applies only to the south curtain workspace')
        split = split_south_walk(workspace, owned, split_report)
        bad += [c for c in split if c['nonmanifold_edges'] or c['world_volume'] <= 0]
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(blend))
    inspection = workspace / 'inspection'
    inspection.mkdir(exist_ok=True)
    actual = {}
    for obj in collection.all_objects:
        if obj.type != 'MESH' or obj.get('asset_group') != asset:
            continue
        node = int(obj['source_node'].split('-')[1])
        key = str(node) + (':' + obj['projection_component'] if obj.get('projection_component') else '')
        m = obj.matrix_world
        actual[key] = {'object': obj.name,
                             'verts': [G.to_native(tuple(m @ v.co)) for v in obj.data.vertices],
                             'faces': [list(p.vertices) for p in obj.data.polygons]}
    (inspection / 'actual-native.json').write_text(json.dumps(actual) + '\n')
    report = {'asset_id': asset, 'recipe': str(Path(__file__).resolve()),
              'recipe_sha256': _sha(__file__), 'helper_sha256': _sha(G.__file__),
              'changed_objects': changes, 'unchanged_nodes': sorted(set(owned) - set(generated)),
              'facts': facts, 'topology_exceptions': bad,
              'world_transform_drift': 0, **split_report}
    (inspection / 'geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    report['mask_revisions'] = apply_mask_revisions(workspace, asset)
    (inspection / 'geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    if packet:
        sys.path.insert(0, str(TOOLING))
        import refinement_workspace
        result = refinement_workspace.modified(str(workspace))
        print(json.dumps(result, indent=2))


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else sys.argv[1:]
    parser = argparse.ArgumentParser()
    parser.add_argument('--asset', required=True, choices=sorted(set(BUILDERS) | set(BUILDERS_R3)))
    parser.add_argument('--no-packet', action='store_true')
    parser.add_argument('--packet-only', action='store_true',
                        help='Keep the workspace geometry (round-2 baselines hold round-1 results); '
                             'export evidence and regenerate the packet only')
    parser.add_argument('--round3', action='store_true', help='Use the round-3 (catalog v3) builders')
    parser.add_argument('--split-140', action='store_true',
                        help='South curtain: split walk 140 into walk and slate-tower floor components')
    parser.add_argument('--only-nodes', help='Comma list of owned node numbers to rebuild (round 2+)')
    parser.add_argument('--preview', help='Plain-Python mode: native JSON of the baseline scene')
    parser.add_argument('--out', help='Plain-Python mode: write generated native shells here')
    args = parser.parse_args(argv)
    if args.preview:
        scene = json.loads(Path(args.preview).read_text())
        base = {int(k): v[0] for k, v in scene.items()}
        generated, facts = BUILDERS[args.asset](base)
        Path(args.out).write_text(json.dumps({str(k): {'verts': s.verts, 'faces': s.faces, 'label': label}
                                              for k, (s, label) in generated.items()}) + '\n')
        print(json.dumps(facts))
        return
    run(args.asset, not args.no_packet, args.packet_only,
        {int(n) for n in args.only_nodes.split(',')} if args.only_nodes else None, args.split_140, args.round3)


if __name__ == '__main__':
    main()
