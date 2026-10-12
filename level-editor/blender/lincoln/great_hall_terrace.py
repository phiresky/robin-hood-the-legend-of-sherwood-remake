"""Detail pass for lincoln-hall-south-terrace: crenellated parapets with real merlons.

Traced evidence: round-1/assets/lincoln-hall-south-terrace/inspection/terrace-parapet-trace.json
(numbered full-image source corners). Every parapet is one continuous wall body per run,
from the terrace ground (native z 400) or the bartizan corbel foot (436) up to the notch
floor, and merlons are part of that body:

- 276  south run: the outer south wall and its parapet. 7 notches, narrow slits about 3 px wide
       (phase from the traced notch shoulders).
- 274  NW parapet above the garden: 2 notches and 3 merlons, traced on its visible inner face.
- 280/279  SW run from the west corner to the bartizan: 2 notches, split at the first notch.
- 277  bartizan: U-shaped ring along the native outer outline with 5 traced merlons and wide
       crenels.

Heights come from the measured merlon-top-front and notch-floor corners of each run:
z = y_front(x) - pixel_y. Nothing else in the asset changes. The floors 243/275, body 278
and stair block 242 keep their native geometry.
"""
import math
from pathlib import Path

from great_hall_geom import crenel_profile, load_trace, replace_mesh

TRACE = (Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement/round-1/assets/'
         'lincoln-hall-south-terrace/inspection/terrace-parapet-trace.json')
GROUND = 400.0          # terrace wall foot (same datum as the pillar cut)
BARTIZAN_FOOT = 436.0   # corbel foot of the hanging bartizan (native 277 bottom; art ~441)


def _sub(a, b):
    return (a[0] - b[0], a[1] - b[1])


def _unit(v):
    n = math.hypot(*v)
    return (v[0] / n, v[1] / n)


def _offset_line(p0, p1, dist, toward):
    """Offset segment p0->p1 by dist along the normal pointing to `toward` (point)."""
    d = _unit(_sub(p1, p0))
    n = (-d[1], d[0])
    mid = ((p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2)
    if (toward[0] - mid[0]) * n[0] + (toward[1] - mid[1]) * n[1] < 0:
        n = (-n[0], -n[1])
    return (p0[0] + n[0] * dist, p0[1] + n[1] * dist), (p1[0] + n[0] * dist, p1[1] + n[1] * dist)


def _offset_polyline(pts, dist, center):
    """Mitred inward offset of an open polyline toward `center`."""
    lines = [_offset_line(pts[i], pts[i + 1], dist, center) for i in range(len(pts) - 1)]
    out = [lines[0][0]]
    for (a0, a1), (b0, b1) in zip(lines, lines[1:]):
        # intersect line a with line b
        da, db = _sub(a1, a0), _sub(b1, b0)
        den = da[0] * db[1] - da[1] * db[0]
        if abs(den) < 1e-9:
            out.append(a1)
            continue
        t = ((b0[0] - a0[0]) * db[1] - (b0[1] - a0[1]) * db[0]) / den
        out.append((a0[0] + da[0] * t, a0[1] + da[1] * t))
    out.append(lines[-1][1])
    return out


def polyline_wall(front, back, ground, floor, merlons):
    """Closed wall along matching front/back polylines (same vertex count).

    Parameter u runs 0..len(front)-1 (segment index + fraction). The profile is split at
    polyline corners so each segment's front/back face is planar; faces share corner
    columns, giving one closed body. Returns (native verts, faces)."""
    n = len(front)
    if len(back) != n or n < 2:
        raise ValueError('front/back polylines must match')
    u1 = n - 1
    prof = crenel_profile(ground, floor, merlons, 0.0, float(u1))
    chain = list(reversed(prof[2:]))  # ascending u top chain
    # insert polyline corners into the top chain
    top = []
    for a, b in zip(chain, chain[1:]):
        top.append(a)
        for k in range(1, u1):
            if a[0] + 1e-9 < k < b[0] - 1e-9:
                z = a[1] + (b[1] - a[1]) * (k - a[0]) / (b[0] - a[0])
                top.append((float(k), z))
    top.append(chain[-1])
    g = ground if callable(ground) else (lambda u, v=ground: v)
    keys, index = [], {}

    def key(u, z):
        k = (round(u, 6), round(z, 4))
        if k not in index:
            index[k] = len(keys)
            keys.append(k)
        return index[k]

    def at(poly, u):
        i = min(int(math.floor(u)), n - 2)
        f = u - i
        a, b = poly[i], poly[i + 1]
        return (a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f)

    fronts = []  # polygons in (u,z) key indices, CCW seen from front
    for s in range(u1):
        seg = [p for p in top if s - 1e-9 <= p[0] <= s + 1 + 1e-9]
        poly = [key(s, g(s)), key(s + 1, g(s + 1))] + [key(u, z) for u, z in reversed(seg)]
        fronts.append(poly)
    m = len(keys)
    verts = []
    for u, z in keys:
        x, y = at(front, u)
        verts.append((x, y, z))
    for u, z in keys:
        x, y = at(back, u)
        verts.append((x, y, z))
    faces = []
    for poly in fronts:
        faces.append(poly)
        faces.append([m + i for i in reversed(poly)])
    tkeys = [key(u, z) for u, z in top]
    if len(keys) != m:
        raise RuntimeError('key table grew after vertex emission')
    for a, b in zip(tkeys, tkeys[1:]):
        ua, ub = keys[a][0], keys[b][0]
        # vertical steps at the wall ends lie in the end caps; do not duplicate them
        if a == b or (abs(ua - ub) < 1e-9 and (abs(ua) < 1e-9 or abs(ua - u1) < 1e-9)):
            continue
        faces.append([a, b, m + b, m + a])
    for s in range(u1):
        a, b = index[(round(float(s), 6), round(g(s), 4))], index[(round(float(s + 1), 6), round(g(s + 1), 4))]
        faces.append([b, a, m + a, m + b])
    for end in (0.0, float(u1)):
        col = [k for k in tkeys if abs(keys[k][0] - end) < 1e-6]
        bot = index[(round(end, 6), round(g(end), 4))]
        ring = [bot] + sorted(set(col), key=lambda k: keys[k][1])
        faces.append(ring + [m + k for k in reversed(ring)])
    return verts, faces


def _mean(vals):
    if not vals:
        raise ValueError('No measured corners for a required height')
    return sum(vals) / len(vals)


def _straight_heights(run, p0, p1):
    def z(c):
        t = (c['px'] - p0[0]) / (p1[0] - p0[0])
        return p0[1] + (p1[1] - p0[1]) * t - c['py']
    tops = [z(c) for c in run['corners'] if c['role'] == 'merlon-top-front' and c['confidence'] == 'measured']
    floors = [z(c) for c in run['corners'] if c['role'] == 'notch-floor' and c['confidence'] == 'measured']
    return (_mean(tops) if tops else None), (_mean(floors) if floors else None)


def _notch_merlons(run, x0, x1, top):
    """Merlons are the stretches between traced notches; returns (xa, xb) pixel-x ranges."""
    lefts = sorted(c['px'] for c in run['corners'] if c['role'] == 'notch-left-shoulder')
    rights = sorted(c['px'] for c in run['corners'] if c['role'] == 'notch-right-shoulder')
    edges = [x0]
    for a, b in zip(lefts, rights):
        edges += [a, b]
    edges.append(x1)
    out = []
    for a, b in zip(edges[0::2], edges[1::2]):
        a, b = max(a, x0), min(b, x1)
        if b - a > 1.0:
            out.append((a, b))
    return out


def apply(by_node):
    trace = load_trace(TRACE)
    runs = {r['id']: r for r in trace['runs']}
    stats = {}

    # South run (276)
    run = runs['S']
    p0, p1 = [tuple(p) for p in run['wall']['front']]
    top, floor = _straight_heights(run, p0, p1)
    b0, b1 = _offset_line(p0, p1, run['wall']['thickness'], (1300.0, 1500.0))
    tx = lambda x: (x - p0[0]) / (p1[0] - p0[0])
    merl = [(tx(a), tx(b), top) for a, b in _notch_merlons(run, p0[0], p1[0], top)]
    v, f = polyline_wall([p0, p1], [b0, b1], GROUND, lambda u: floor, merl)
    stats['building-276'] = {**replace_mesh(by_node['building-276'], v, f),
                             'merlons': len(merl), 'notches': sum(c['role'] == 'notch-left-shoulder' for c in run['corners']),
                             'merlon_top_z': round(top, 1), 'notch_floor_z': round(floor, 1)}
    s_floor, s_top = floor, top

    # NW parapet (274): trace on its visible inner (south) face.
    run = runs['NW']
    p0, p1 = [tuple(p) for p in run['wall']['front']]
    q0, q1 = [tuple(p) for p in run['wall']['back']]
    # The native back edge starts ~5 px further west than the front; the reviewed mask 322
    # and the artwork end the west corner at the front edge's x (coverage audit showed a
    # 3-5 px visible-but-rejected strip there), so start the back edge at the same x.
    tq = (p0[0] - q0[0]) / (q1[0] - q0[0])
    q0 = (p0[0], q0[1] + (q1[1] - q0[1]) * tq)
    top, floor = _straight_heights(run, p0, p1)
    tx = lambda x: (x - p0[0]) / (p1[0] - p0[0])
    rng = _notch_merlons(run, p0[0], p1[0], top)
    rend = max(c['px'] for c in run['corners'] if c['role'] == 'merlon-right-shoulder')
    rng[-1] = (rng[-1][0], min(rng[-1][1], rend))
    merl = [(tx(a), tx(b), top) for a, b in rng]
    v, f = polyline_wall([p0, p1], [q0, q1], GROUND, lambda u: floor, merl)
    stats['building-274'] = {**replace_mesh(by_node['building-274'], v, f),
                             'merlons': len(merl), 'notches': len(merl) - 1,
                             'merlon_top_z': round(top, 1), 'notch_floor_z': round(floor, 1)}

    # SW run (280 west part, 279 rest), split at the first notch's left shoulder.
    run = runs['SW']
    b0, b1 = [tuple(p) for p in run['wall']['back']]
    f0, f1 = _offset_line(b0, b1, run['wall']['thickness'], (1150.0, 1600.0))
    top, _ = _straight_heights(run, f0, f1)
    floor = s_floor  # SW notch floors are in shadow; use the measured south-run floor (inferred)
    tx = lambda x: (x - f0[0]) / (f1[0] - f0[0])
    rng = _notch_merlons(run, f0[0], f1[0], top)
    split_x = min(c['px'] for c in run['corners'] if c['role'] == 'notch-left-shoulder')
    ts = tx(split_x)
    lerp = lambda a, b, t: (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
    fs, bs = lerp(f0, f1, ts), lerp(b0, b1, ts)
    for node, (fa, fb, ba, bb, lo, hi) in {'building-280': (f0, fs, b0, bs, 0.0, ts),
                                            'building-279': (fs, f1, bs, b1, ts, 1.0)}.items():
        local = []
        for a, b in rng:
            ta, tb = max(tx(a), lo), min(tx(b), hi)
            if tb - ta > 1e-3:
                local.append(((ta - lo) / (hi - lo), (tb - lo) / (hi - lo), top))
        v, f = polyline_wall([fa, fb], [ba, bb], GROUND, lambda u: floor, local)
        stats[node] = {**replace_mesh(by_node[node], v, f), 'merlons': len(local),
                       'merlon_top_z': round(top, 1), 'notch_floor_z': round(floor, 1),
                       'notch_floor_source': 'south-run measured floor (SW notch floors shadowed)'}

    # Bartizan (277)
    run = runs['BARTIZAN']
    outer = [tuple(p) for p in run['wall']['outer']]
    center = (1246.0, 1582.0)
    inner = _offset_polyline(outer, run['wall']['thickness'], center)

    def seg_u(px, seg_hint):
        # visible outer segments are monotonic in x; choose by hint (first listed segment)
        segs = [int(s) for s in seg_hint.replace('seg', '').split('-')]
        for s in range(min(segs), max(segs) + 1):
            a, b = outer[s], outer[s + 1]
            lo, hi = sorted((a[0], b[0]))
            if lo - 1e-6 <= px <= hi + 1e-6:
                return s + (px - a[0]) / (b[0] - a[0])
        raise ValueError(f'pixel x {px} not on bartizan segments {seg_hint}')

    def seg_z(c):
        u = seg_u(c['px'], c['segment'])
        s = min(int(u), len(outer) - 2)
        f_ = u - s
        y = outer[s][1] + (outer[s + 1][1] - outer[s][1]) * f_
        return y - c['py']

    floors = [seg_z(c) for c in run['corners'] if c['role'] == 'notch-floor']
    bfloor = _mean(floors)
    merl = []
    for i in sorted({c['merlon'] for c in run['corners'] if 'merlon' in c}):
        cs = {c['role']: c for c in run['corners'] if c.get('merlon') == i}
        a, b = cs['merlon-left-shoulder'], cs['merlon-right-shoulder']
        ua, ub = sorted((seg_u(a['px'], a['segment']), seg_u(b['px'], b['segment'])))
        merl.append((ua, ub, seg_z(cs['merlon-top'])))
    merl.sort()
    btop = _mean([m[2] for m in merl])
    v, f = polyline_wall(outer, inner, BARTIZAN_FOOT, lambda u: bfloor, merl)
    stats['building-277'] = {**replace_mesh(by_node['building-277'], v, f), 'merlons': len(merl),
                             'merlon_top_z': round(btop, 1), 'merlon_tops_z': [round(m[2], 1) for m in merl],
                             'notch_floor_z': round(bfloor, 1), 'foot_z': BARTIZAN_FOOT}
    return stats
