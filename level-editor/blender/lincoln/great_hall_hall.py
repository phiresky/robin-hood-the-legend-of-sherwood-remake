"""Detail pass for lincoln-great-hall: east-wing roof envelope, stepped SE gable,
middle-tower and west-wing parapet merlons, and the west roof base slab.

All measurements are original covered.png pixels, stored with their provenance in
``TRACE`` (written to the workspace's inspection/hall-trace.json by write_trace()).
"""
import json
from pathlib import Path

import great_hall_geom as _geom
Vector = _geom.Vector

import great_hall_geom as G

WORKSPACE = Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement/round-1/assets/lincoln-great-hall'

# --- East wing -------------------------------------------------------------------------
# Native roof lines (from the native roof receivers 246/247; their SE-end projection
# matches the gable eaves at (1466,909)/(1687,851) px and the apex at (1584,795) px).
RIDGE = ((1427.0, 1384.5, 730.0), (1584.0, 1525.0, 730.0))
SW_EAVE = (1309.0, 1428.0, 659.0)
NE_EAVE = (1531.0, 1346.0, 636.0)

# SE gable slab (268 footprint): outer (visible SE face) and inner lines.
GABLE_OUT = ((1482.0, 1562.0), (1678.0, 1490.0))
GABLE_IN = ((1469.0, 1550.0), (1665.0, 1478.0))
GABLE_BODY_TOP = 630.0
# Crow-step front-left corners on the SW rake, traced at 6x (inspection/east-gable-trace.png).
GABLE_STEPS_SW = [  # (id, px, py) front-left corner of each step's top face
    (1, 1469, 893), (2, 1490, 876), (3, 1504, 859), (4, 1520, 841),
    (5, 1534, 824), (6, 1550, 806), (7, 1562, 791)]
# NE rake: only partly readable (keep-annex shadow); two blocks measured, the rest mirror
# the SW pitch (inferred).
GABLE_STEPS_NE_MEASURED = [(8, 1605, 797), (9, 1650, 812), (10, 1685, 840)]

# --- Middle tower parapet (260 ring, 732-759) -----------------------------------------
TOWER = {
    # wall: (outer p0, outer p1, inner q0, inner q1, notch centre pixel xs, confidence)
    'SW': ((1277, 1475), (1330, 1522), (1289, 1476), (1335, 1516), [1292, 1306, 1320], ['measured', 'measured', 'inferred']),
    'SE': ((1330, 1522), (1410, 1493), (1335, 1516), (1397, 1491), [1345, 1371, 1396], ['measured', 'measured', 'inferred']),
    'NE': ((1410, 1493), (1357, 1445), (1397, 1491), (1354, 1452), [1397, 1384, 1370], ['inferred'] * 3),
    'NW': ((1357, 1445), (1277, 1475), (1354, 1452), (1289, 1476), [1338, 1317, 1297], ['inferred'] * 3),
}
TOWER_BASE, TOWER_TOP, TOWER_FLOOR, TOWER_NOTCH_PX = 732.0, 759.0, 747.0, 7.0

# --- West wing SW eave parapet (261) ---------------------------------------------------
PARAPET_SW = ((1037.1, 1337.4, 653.0), (1225.7, 1500.5, 651.1))   # outer line with native top
PARAPET_SW_IN = ((1042.0, 1334.8), (1226.0, 1494.1))
PARAPET_SE = ((1225.7, 1500.5, 651.1), (1297.5, 1473.5, 651.4))
PARAPET_SE_IN = ((1226.0, 1494.1), (1292.7, 1469.3))
# Visible SE end faces of merlons (dark shadowed faces) traced at 6x.
PARAPET_MERLON_RIGHT = [
    (10, 1051.5, 'measured'), (11, 1065.0, 'measured'), (12, 1082.0, 'inferred'),
    (13, 1099.0, 'measured'), (14, 1112.0, 'inferred'), (15, 1129.3, 'inferred'),
    (16, 1146.5, 'measured'), (17, 1164.0, 'measured'), (18, 1180.5, 'measured'),
    (19, 1198.5, 'measured'), (20, 1215.5, 'measured')]
PARAPET_MERLON_W, PARAPET_UP, PARAPET_DOWN = 10.0, 3.0, 8.0

# West roof base slab 271: clipped to the actual eave outline of 248/249/250/251.
ROOF_BASE = [(1003, 1300), (1026, 1328), (1226, 1502), (1442, 1421), (1244, 1247),
             (1162, 1213), (1137, 1207), (1022, 1247), (997, 1254)]


def _plane(a, b, c, up=True):
    p, n = G.plane_from_points(a, b, c)
    if up and n.z < 0:
        n = -n
    return p, n


def east_roof_planes():
    sw = _plane(RIDGE[0], RIDGE[1], SW_EAVE)
    ne = _plane(RIDGE[0], RIDGE[1], NE_EAVE)
    return sw, ne


def gable_steps():
    (o0, o1) = GABLE_OUT
    steps = []
    for sid, px, py in GABLE_STEPS_SW:
        t, z = G.pixel_to_tz(o0, o1, px, py)
        steps.append((max(t, 0.0), z, sid, 'measured'))
    t_apex = (RIDGE[1][0] - o0[0]) / (o1[0] - o0[0])
    pitch_t = (steps[-1][0] - steps[0][0]) / (len(steps) - 1)
    rise = (steps[-1][1] - steps[0][1]) / (len(steps) - 1)
    # The top step straddles the apex; NE steps mirror the SW pitch and descend.
    top_end = 2 * t_apex - steps[-1][0]
    # NE step tops follow a line fitted through the measured NE blocks (the NE rake
    # descends more steeply than a mirror of the SW rake); step pitch mirrors the SW pitch.
    pts = [G.pixel_to_tz(o0, o1, px, py) for _, px, py in GABLE_STEPS_NE_MEASURED]
    n = len(pts)
    mt = sum(t for t, _ in pts) / n
    mz = sum(z for _, z in pts) / n
    slope = sum((t - mt) * (z - mz) for t, z in pts) / sum((t - mt) ** 2 for t, _ in pts)
    ne = []
    t, k = top_end, 0
    while t < 1.0 - 1e-6:
        z = min(mz + slope * (t - mt), steps[-1][1] - 1.0)
        ne.append((t, max(z, 640.0), 100 + k, 'inferred-pitch/fitted-height'))
        t += pitch_t
        k += 1
    return steps, ne, t_apex


def gable_profiles():
    sw, ne, t_apex = gable_steps()
    # Staircase over [0, 1]: SW steps rise, NE steps fall.
    starts = [(t, z) for t, z, _, _ in sw] + [(t, z) for t, z, _, _ in ne]
    top = []
    for i, (t, z) in enumerate(starts):
        t_next = starts[i + 1][0] if i + 1 < len(starts) else 1.0
        top.append((t, z))
        top.append((t_next, z))
    def part(t0, t1):
        pts = []
        for i in range(0, len(top), 2):
            a, b = top[i][0], top[i + 1][0]
            z = top[i][1]
            lo, hi = max(a, t0), min(b, t1)
            if hi - lo > 1e-6:
                pts += [(lo, z), (hi, z)]
        return [(t0, GABLE_BODY_TOP), (t1, GABLE_BODY_TOP)] + list(reversed(pts))
    return part(0.0, t_apex), part(t_apex, 1.0), t_apex, sw, ne


def _notched(p0, p1, q0, q1, base, floor, merlons):
    prof = G.crenel_profile(base, floor, merlons)
    return G.notched_wall(p0, p1, q0, q1, prof)


def tower_wall(key):
    p0, p1, q0, q1, notches, _ = TOWER[key]
    span = p1[0] - p0[0]
    half = TOWER_NOTCH_PX / 2 / abs(span)
    centres = sorted((x - p0[0]) / span for x in notches)
    merlons, cur = [], 0.0
    for c in centres:
        merlons.append((cur, c - half, TOWER_TOP))
        cur = c + half
    merlons.append((cur, 1.0, TOWER_TOP))
    return _notched(p0, p1, q0, q1, TOWER_BASE, lambda t: TOWER_FLOOR, merlons)


def parapet_walls(ground):
    (a, b), (qa, qb) = PARAPET_SW, PARAPET_SW_IN
    p0, p1 = a[:2], b[:2]
    top = lambda t: a[2] + (b[2] - a[2]) * t
    floor = lambda t: top(t) - PARAPET_DOWN
    span = p1[0] - p0[0]
    merlons = []
    for _, x, _ in PARAPET_MERLON_RIGHT:
        tb = (x - p0[0]) / span
        ta = (x - PARAPET_MERLON_W - p0[0]) / span
        merlons.append((max(ta, 0.0), min(tb, 1.0), lambda t, f=top: f(t) + PARAPET_UP))
    merlons.append(((1225.7 - 4 - p0[0]) / span, 1.0, lambda t, f=top: f(t) + PARAPET_UP))  # corner merlon
    sw = _notched(p0, p1, qa, qb, ground, floor, merlons)
    (c, d), (qc, qd) = PARAPET_SE, PARAPET_SE_IN
    top2 = lambda t: c[2] + (d[2] - c[2]) * t
    span2 = d[0] - c[0]
    m2, x = [], c[0]
    # SE run: behind the terrace; merlon phase continues the SW period (inferred).
    period = 17.2
    while x < d[0] - 1e-6:
        ta, tb = (x - c[0]) / span2, min((x + PARAPET_MERLON_W - c[0]) / span2, 1.0)
        m2.append((ta, tb, lambda t, f=top2: f(t) + PARAPET_UP))
        x += period
    se = _notched(c[:2], d[:2], qc, qd, ground, lambda t: top2(t) - PARAPET_DOWN, m2)
    return G.merge([sw, se]), len(merlons), len(m2)


def apply(by_node):
    out = {}
    need = [f'building-{n}' for n in (241, 245, 260, 261, 268, 270, 271, 272)]
    missing = [n for n in need if n not in by_node]
    if missing:
        raise ValueError(f'great hall detail: missing owned nodes {missing}')
    hall_ground = min(G.to_native(by_node['building-268'].matrix_world @ v.co)[2]
                      for v in by_node['building-268'].data.vertices)
    west_ground = min(G.to_native(by_node['building-261'].matrix_world @ v.co)[2]
                      for v in by_node['building-261'].data.vertices)

    # 1. East-wing roof envelope: flat-topped sight slabs 270/272 must not rise above the
    #    gable roof planes (272 projected into the keep tower pixels).
    (psw, nsw), (pne, nne) = east_roof_planes()
    for node in ('building-270', 'building-272'):
        s1 = G.clip_above_plane(by_node[node], psw, nsw)
        s2 = G.clip_above_plane(by_node[node], pne, nne)
        out[node] = {'clipped_to_roof_planes': True, **s1, 'clip_caps_ne': s2['clip_caps']}

    # 1b. Nothing of the east-wing roof or its eave slab 269 may project beyond the SE gable
    #     face: the art shows a plain stepped gable wall there (269 painted a band across it).
    (ga, gb) = GABLE_OUT
    outward = Vector((gb[1] - ga[1], -(gb[0] - ga[0]), 0.0)).normalized()
    if outward.y < 0:
        outward = -outward
    for node in ('building-246', 'building-247', 'building-269'):
        if node not in by_node:
            raise ValueError(f'great hall detail: missing {node}')
        out[node] = {'clipped_to_gable_face': True,
                     **G.clip_above_plane(by_node[node], (ga[0], ga[1], 600.0), outward)}

    # 2. Stepped SE gable: 268 body (ground..630), 245 SW half, 241 NE half above 630.
    sw_prof, ne_prof, t_apex, sw_steps, ne_steps = gable_profiles()
    body = G.notched_wall(GABLE_OUT[0], GABLE_OUT[1], GABLE_IN[0], GABLE_IN[1],
                          [(0.0, hall_ground), (1.0, hall_ground), (1.0, GABLE_BODY_TOP), (0.0, GABLE_BODY_TOP)])
    out['building-268'] = G.replace_mesh(by_node['building-268'], *body)
    lo = lambda P, t: G.lerp2(P[0], P[1], t)
    for node, prof, t0, t1 in (('building-245', sw_prof, 0.0, t_apex), ('building-241', ne_prof, t_apex, 1.0)):
        v, f = G.notched_wall(GABLE_OUT[0], GABLE_OUT[1], GABLE_IN[0], GABLE_IN[1], prof)
        out[node] = {**G.replace_mesh(by_node[node], v, f), 'steps': sum(1 for _ in prof) // 2 - 1}

    # 3. Middle-tower parapet ring with merlons.
    tower = G.merge([tower_wall(k) for k in ('SW', 'SE', 'NE', 'NW')])
    out['building-260'] = {**G.replace_mesh(by_node['building-260'], *tower),
                           'merlons_per_wall': 4, 'notches_per_wall': 3}

    # 4. West-wing SW eave parapet 261 with merlons (continuous wall body from ground).
    walls, n_sw, n_se = parapet_walls(west_ground)
    out['building-261'] = {**G.replace_mesh(by_node['building-261'], *walls),
                           'merlons_sw_run': n_sw, 'merlons_se_run': n_se}

    # 5. West roof base slab 271 clipped to the traced eave outline (removes NW/N overhang).
    slab = G.prism(ROOF_BASE, 629.0, 636.0)
    out['building-271'] = G.replace_mesh(by_node['building-271'], *slab)
    write_trace(sw_steps, ne_steps)
    return out


def write_trace(sw_steps=None, ne_steps=None):
    import hashlib
    trace = {'version': 1, 'asset_id': 'lincoln-great-hall',
             'source': str(G.SOURCE), 'source_sha256': hashlib.sha256(G.SOURCE.read_bytes()).hexdigest(),
             'note': 'Original full-image covered.png pixels. Front (outer, SE/SW-facing) cap edges traced.',
             'runs': []}
    trace['runs'].append({'id': 'east-gable-sw-rake', 'source_node': 'building-245', 'role': 'crow-step front-left corners',
                          'wall_outer_edge': GABLE_OUT, 'corners': [
                              {'id': i, 'px': x, 'py': y, 'role': 'step-front-left', 'confidence': 'measured'}
                              for i, x, y in GABLE_STEPS_SW]})
    trace['runs'].append({'id': 'east-gable-ne-rake', 'source_node': 'building-241', 'role': 'step blocks',
                          'wall_outer_edge': GABLE_OUT, 'corners': [
                              {'id': i, 'px': x, 'py': y, 'role': 'step-block', 'confidence': 'measured-low'}
                              for i, x, y in GABLE_STEPS_NE_MEASURED],
                          'inferred_steps_tz': [[round(t, 4), round(z, 1)] for t, z, _, _ in (ne_steps or [])]})
    for key, (p0, p1, q0, q1, xs, conf) in TOWER.items():
        trace['runs'].append({'id': f'tower-{key}', 'source_node': 'building-260', 'role': 'notch centres (dark slits)',
                              'wall_outer_edge': [p0, p1], 'corners': [
                                  {'id': 30 + 3 * list(TOWER).index(key) + k, 'px': x,
                                   'py': round(p0[1] + (p1[1] - p0[1]) * (x - p0[0]) / (p1[0] - p0[0]) - TOWER_TOP, 1),
                                   'role': 'notch-centre', 'confidence': c} for k, (x, c) in enumerate(zip(xs, conf))]})
    a, b = PARAPET_SW
    trace['runs'].append({'id': 'west-wing-sw-parapet', 'source_node': 'building-261', 'role': 'merlon SE end faces',
                          'wall_outer_edge': [a[:2], b[:2]], 'corners': [
                              {'id': i, 'px': x, 'py': round(a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0])
                                                             - (a[2] + (b[2] - a[2]) * (x - a[0]) / (b[0] - a[0])) - PARAPET_UP, 1),
                               'role': 'merlon-right-end', 'confidence': c} for i, x, c in PARAPET_MERLON_RIGHT]})
    (WORKSPACE / 'inspection').mkdir(exist_ok=True)
    (WORKSPACE / 'inspection/hall-trace.json').write_text(json.dumps(trace, indent=1) + '\n')
    return trace
