"""Detail pass for lincoln-hall-approach-ramp (great_hall lane).

287: parapet rebuilt as one continuous notched wall from the numbered corner trace
     (inspection/ramp-corner-trace.json): 13 merlons, notch floor and merlon tops from the
     traced shadowed merlon end faces. The native top (336 -> 419) lies ~10 below the drawn
     notch floor, so the parapet floor follows the traced corners.
288: walkway rebuilt as five sloped landings separated by four low risers at the traced
     step lines (parallel to the lower end edge), plus the native top landing wedge that
     rises to 420 at the door/pier end.
286: pier kept (ground cut only).
apply(by_node) returns {source_node: stats}.
"""
from pathlib import Path

import great_hall_geom as G

TRACE = (Path(__file__).resolve().parents[2] /
         'work/needs-review/lincoln-refinement/round-1/assets/lincoln-hall-approach-ramp/inspection/ramp-corner-trace.json')

GROUND = 300.0
# Native footprint of 287 (baseline): front (south) face and back face edges.
P287 = ((1223.6, 1674.1), (1493.6, 1573.5))
Q287 = ((1221.1, 1671.9), (1491.2, 1571.4))
Z287 = (336.0, 419.0)          # native top at the two ends (baseline)
# Native walkway 288: SW edge (behind the parapet) and NE edge (terrace side).
P288 = ((1221.7, 1671.3), (1490.3, 1570.1))
Q288 = ((1159.7, 1617.3), (1427.9, 1516.2))
Z288 = (330.0, 414.6)
RISER = 3.0                    # riser shadow lines are 2-3 px thick -> ~3 native units (inferred)
# Top landing wedge (native baseline top face): rises from 414.6 to 420 towards the pier 286.
WEDGE = [((1427.9, 1516.2), 414.6), ((1453.3, 1516.8), 419.9),
         ((1483.9, 1543.9), 419.8), ((1490.3, 1570.1), 414.6)]


def _ytop(t):
    return P287[0][1] + (P287[1][1] - P287[0][1]) * t


def parapet_profile(trace):
    run = next(r for r in trace['runs'] if r['id'] == 'parapet-287')
    c = run['corners']
    merlons, floors = [], []
    for i in range(0, len(c), 4):
        fl, tl, tr, fr = c[i:i + 4]
        ta, zfl = G.pixel_to_tz(P287[0], P287[1], fl['px'], fl['py'])
        _, ztl = G.pixel_to_tz(P287[0], P287[1], tl['px'], tl['py'])
        tb, _ = G.pixel_to_tz(P287[0], P287[1], tr['px'], tr['py'])
        base = Z287[0] + (Z287[1] - Z287[0]) * ta
        floors.append(zfl - base)
        height = ztl - zfl
        merlons.append((ta, tb, height))
    floor_off = sorted(floors)[len(floors) // 2]      # median traced floor offset above native top

    def floor(t):
        return Z287[0] + (Z287[1] - Z287[0]) * t + floor_off
    prof_merlons = [(ta, tb, (lambda t, h=h: floor(t) + h)) for ta, tb, h in merlons]
    profile = G.crenel_profile(GROUND, floor, prof_merlons)
    return profile, {'merlons': len(merlons), 'floor_offset_above_native_top': round(floor_off, 2),
                     'merlon_heights': [round(h, 1) for _, _, h in merlons],
                     'merlon_t': [[round(a, 4), round(b, 4)] for a, b, _ in merlons]}


def walkway(trace):
    run = next(r for r in trace['runs'] if r['id'] == 'walkway-steps-288')
    ts = [c['t'] for c in run['corners']]
    slope = (Z288[1] - Z288[0] - RISER * len(ts))
    def z(t, k):
        return Z288[0] + RISER * k + slope * t
    top = [(0.0, z(0.0, 0))]
    for k, t in enumerate(ts):
        top.append((t, z(t, k)))
        top.append((t, z(t, k + 1)))
    top.append((1.0, z(1.0, len(ts))))
    profile = [(0.0, GROUND), (1.0, GROUND)] + list(reversed(top))
    mv, mf = G.notched_wall(P288[0], P288[1], Q288[0], Q288[1], profile)
    # Profile index 1 -> 2 is the upper end face (t = 1); the wedge continues there.
    mf = [face for k, face in enumerate(mf) if k != 2 + 1]
    # Wedge: vertical prism over the landing polygon with its native sloped top.
    poly = WEDGE
    n = len(poly)
    verts = [(x, y, GROUND) for (x, y), _ in poly] + [(x, y, zt) for (x, y), zt in poly]
    faces = [list(range(n - 1, -1, -1)), list(range(n, 2 * n))]
    for i in range(n - 1):              # side 3 -> 0 is shared with the main slab end face
        faces.append([i, i + 1, n + i + 1, n + i])
    return G.merge([(mv, mf), (verts, faces)]), {'landings': len(ts) + 1, 'risers': len(ts),
                                             'riser_height': RISER, 'step_t': ts}


def apply(by_node):
    trace = G.load_trace(TRACE)
    out = {}
    profile, info = parapet_profile(trace)
    v, f = G.notched_wall(P287[0], P287[1], Q287[0], Q287[1], profile)
    out['building-287'] = {**info, **G.replace_mesh(by_node['building-287'], v, f)}
    (v, f), info = walkway(trace)
    # The wedge shares the upper end edge with the main slab; the weld closes the seam.
    out['building-288'] = {**info, **G.replace_mesh(by_node['building-288'], v, f, weld=0.3)}
    return out
