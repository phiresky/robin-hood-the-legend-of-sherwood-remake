"""Lincoln keep_north lane geometry recipe (keep, annex, turrets, north hall, curtains).

Run in background Blender on one asset workspace, never on frozen evidence:

  /usr/bin/blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/refine_keep_north.py -- \
      --asset lincoln-keep [--packet]

The recipe opens ``round-1/assets/<asset>/model.blend``, rebuilds only the owned
meshes from the measured specification below, saves, and with ``--packet``
regenerates ``modified/`` through the frozen tooling snapshot.  It is idempotent:
every owned mesh is rebuilt from the specification, never from its current state.

Plain python (no Blender) with ``--preview`` writes a source-camera painter
preview of the specification to the asset's ``inspection/`` directory.

Coordinates are native map units: the 35-degree source camera maps (x, y, z) to
pixel (x, y - z).  Measurements come from enlarged, gridded crops of
``source-states/covered.png``; see each asset's review.md and inspection/ JSON.
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import keep_north_geom as G  # noqa: E402

ROOT = HERE.parents[2]
REFINE = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
ASSETS = REFINE / 'round-1/assets'  # --assets-dir selects round-2
TOOLING = REFINE / 'tooling/e6b57cb851c7142b'
GROUND = G.GROUND


def node(n):
    return f'building-{n:03d}'


# ================================================================== keep

# Tall crenellated tower standing on the terrace (pixel crown x 1640-1758).
TALL = dict(cx=1697.0, cy=1153.5, r=56.0, crown_out=59.0, crown_in=50.5,
            base=796.0, crown_base=1026.0, floor=1036.0, notch=1046.0, top=1061.0,
            merlons=13, phase=93.0, merlon_deg=13.6)
# Flat-topped round tower rising from the plateau through the keep.
ROUND = dict(cx=1760.0, cy=1116.0, r=97.0, rim_out=101.0, rim_in=90.0,
             floor=892.0, rim_top=916.0)
# Slate-spire turret on the south-west keep corner.
SPIRE = dict(cx=1565.5, cy=1266.0, r=61.0, eave_r=72.0, eave=839.0, apex=973.0,
             pole_top=1149.0, pole_r=2.0)


def keep_spec():
    so = native_obstacles()
    pts = lambda i: [(p['x'], p['y']) for p in so[i]['points']]
    out = {}

    def M(n):
        out.setdefault(node(n), G.Mesh())
        return out[node(n)]

    # 190: solid lower storey below the revealed room floor (was 0..550).
    G.prism(M(190), pts(190), GROUND, 550.0)
    # 209: terrace slab (native 794..800, unchanged footprint).
    G.prism(M(209), pts(209), 794.0, 800.0)
    # 210: north/west outer wall band, plain parapet (artwork shows no merlons).
    G.prism(M(210), pts(210), GROUND, 828.0)
    # 215/216: interior wall linings of the revealed room stand on its floor.
    G.prism(M(215), pts(215), 550.0, 756.0)
    G.prism(M(216), pts(216), 550.0, 755.0)
    # 217 upper / 460 lower west gap pieces (460 is disabled by Patch06).  The
    # native slabs were set back from the outer face, leaving a visible recess
    # and an open corner; they now span flush between the 210 and 226 wall ends.
    p210, p226, p227 = pts(210), pts(226), pts(227)
    west_gap = [p210[0], p210[9], p226[8], p226[7]]
    G.prism(M(217), west_gap, 620.0, 828.0)
    G.prism(M(460), west_gap, GROUND, 623.0)
    # 207 upper / 459 lower front gap pieces (459 is disabled by Patch06), flush
    # between the 226 and 227 wall ends.
    front_gap = [p226[17], p226[0], p227[1], p227[2]]
    G.prism(M(207), front_gap, 608.0, 829.0)
    G.prism(M(459), front_gap, GROUND, 627.0)
    # 226: south front wall; the turret ring is replaced by a true cylinder.
    west = [p226[7], p226[8], p226[9], p226[6]]
    east = [p226[13], p226[14], p226[15], p226[16], p226[17], p226[0], p226[1], p226[2]]
    G.prism(M(226), west, GROUND, 827.0)
    G.prism(M(226), east, GROUND, 827.0)
    s = SPIRE
    G.cylinder(M(226), s['cx'], s['cy'], s['r'], GROUND, s['eave'] + 3.0, n=48)
    # 227: east parapet band; artwork parapet top ~830 at the SE corner.
    G.prism(M(227), pts(227), GROUND, 830.0)
    # 221: small pent hood above the front slit window.
    p221 = pts(221)
    G.prism(M(221), p221, 684.0, [713.0, 692.0, 692.0, 713.0])
    # 218/219: slate spire split along x = cx into west and east halves.
    G.cone(M(218), s['cx'], s['cy'], s['eave_r'], s['eave'], s['apex'], n=48, start=90, end=270)
    G.cone(M(219), s['cx'], s['cy'], s['eave_r'], s['eave'], s['apex'], n=48, start=-90, end=90)
    G.cylinder(M(218), s['cx'], s['cy'], s['pole_r'], s['apex'] - 6.0, s['pole_top'], n=8)
    # 208/212: round tower body and its plain rim parapet.
    r = ROUND
    G.cylinder(M(208), r['cx'], r['cy'], r['r'], GROUND, r['floor'], n=64)
    G.ring(M(212), r['cx'], r['cy'], r['rim_out'], r['rim_in'], r['floor'] - 6.0, r['rim_top'], n=64)
    # 213/211/214: tall tower body, crenellated crown, crown floor and door face.
    t = TALL
    G.cylinder(M(213), t['cx'], t['cy'], t['r'], t['base'], t['crown_base'] + 2.0, n=48)
    G.crenellated_ring(M(213), t['cx'], t['cy'], t['crown_out'], t['crown_in'], t['crown_base'],
                       t['notch'], t['top'], t['merlons'], t['phase'], t['merlon_deg'])
    G.cylinder(M(211), t['cx'], t['cy'], t['crown_in'] + 0.5, t['floor'] - 8.0, t['floor'], n=48)
    shell = M(214).new()
    angles = [55 + 70 * i / 12 for i in range(13)]
    for a0, a1 in zip(angles, angles[1:]):
        quad = [G.circ(t['cx'], t['cy'], t['r'] + 0.6, a0, 0)[:2], G.circ(t['cx'], t['cy'], t['r'] + 0.6, a1, 0)[:2],
                G.circ(t['cx'], t['cy'], t['r'] - 4.0, a1, 0)[:2], G.circ(t['cx'], t['cy'], t['r'] - 4.0, a0, 0)[:2]]
        shell.cell([(x, y, 850.0) for x, y in quad], [(x, y, t['crown_base']) for x, y in quad])
    # Interior furniture stands on the room floor (was extruded from z = 0).
    for n, top in ((326, 571.0), (328, 572.0), (329, 558.0), (348, 563.0)):
        G.prism(M(n), pts(n), 550.0, top)
    G.prism(M(327), pts(327), 567.0, 570.0)
    return out



def _thickness(a, b, c):
    """World distance from native point c to the native line a-b."""
    ax, ay = a[0], -a[1] / G.SIN
    bx, by = b[0], -b[1] / G.SIN
    cx, cy = c[0], -c[1] / G.SIN
    dx, dy = bx - ax, by - ay
    return abs((cx - ax) * dy - (cy - ay) * dx) / (dx * dx + dy * dy) ** 0.5


EVIDENCE = {}


def crenel_run(out_mesh, key, path, inside, spans, notch, top, zproj, thickness=None):
    """Merlon blocks for one measured parapet run; records the corner evidence."""
    t = thickness if thickness is not None else _thickness(path[0], path[-1], inside)
    placed = G.path_blocks(out_mesh, path, t, inside, spans, notch - 0.5, top, zproj)
    EVIDENCE[key] = {'path_native': [list(p) for p in path], 'inside_native': list(inside),
                     'thickness_world': round(t, 3), 'notch_z': notch, 'top_z': top,
                     'corner_projection_z': zproj, 'merlons': placed}


# ================================================================== annex

def annex_spec():
    so = native_obstacles()
    pts = lambda i: [(p['x'], p['y']) for p in so[i]['points']]
    out = {}

    def M(n):
        out.setdefault(node(n), G.Mesh())
        return out[node(n)]

    EVIDENCE.clear()
    G.prism(M(189), pts(189), GROUND, 472.0)          # lower storey below the Patch07 room
    G.prism(M(198), pts(198), 545.0, 550.0)           # raised wall-walk slab
    # Round 2 (user: "just look at it"): the walk slab was carried only by thin parapet
    # walls, leaving the north-east arm and the west wing hollow down to the courtyard.
    # Solid masonry cores now fill the terrace outside the Patch07 room block 189.
    # North-east arm: between parapet walls 202 (west) and 201 (east), from the keep side
    # down to the north edge of 189.
    ARM = [(1733.6, 1251.9), (1788.8, 1231.0), (1878.7, 1367.3), (1818.0, 1380.4)]
    # West wing: between the south-west parapet 203, the front-left parapet 204 and 189.
    WING = [(1613.6, 1458.4), (1818.0, 1380.4), (1726.4, 1399.8), (1732.0, 1469.0), (1644.0, 1486.6)]
    G.prism(M(198), ARM, GROUND, 545.0)
    G.prism(M(198), WING, GROUND, 545.0)
    # 201: east parapet of the north-east arm; 13 lit merlon end faces were
    # located from a brightness profile along the run (see inspection JSON).
    lit = [1800.5, 1811, 1819, 1827, 1838, 1847, 1856, 1864.5, 1874, 1882.5, 1891.5, 1900, 1908.5]
    G.prism(M(201), pts(201), GROUND, 557.0)
    crenel_run(M(201), '201', [pts(201)[0], pts(201)[1]], pts(201)[2],
               [(e - 2.5, e + 2.5) for e in lit], 557.0, 567.0, 567.0)
    # 202: west parapet of the arm (ten merlons plus the corner block).
    G.prism(M(202), pts(202), GROUND, 563.0)
    crenel_run(M(202), '202', [pts(202)[3], pts(202)[2]], pts(202)[0],
               [(1736.5, 1741.5), (1746.5, 1750.5), (1754.5, 1759.5), (1762.5, 1767.5),
                (1771.5, 1776.5), (1779.5, 1784.5), (1787.5, 1792.5), (1796.5, 1801.5),
                (1805.5, 1810.5), (1813.0, 1820.0)], 563.0, 573.0, 573.0)
    # 203: south-west parapet facing the keep; x < 1680 is hidden by the great hall.
    G.prism(M(203), pts(203), GROUND, 566.0)
    crenel_run(M(203), '203', [pts(203)[2], pts(203)[1]], pts(203)[0],
               [(1681, 1688), (1697, 1706), (1717, 1727), (1737, 1746), (1758, 1767),
                (1781, 1791), (1805, 1818)], 566.0, 576.0, 576.0)
    # 204: front-left parapet; 205 is the tall pier at its east end.
    G.prism(M(204), pts(204), GROUND, 565.0)
    crenel_run(M(204), '204', [pts(204)[2], pts(204)[1]], pts(204)[0],
               [(1680, 1693), (1705, 1718), (1727, 1735)], 565.0, 573.0, 573.0)
    G.prism(M(458), pts(458), GROUND, 533.0)          # Patch07 cover (kept)
    G.prism(M(205), pts(205), 531.0, 580.0)
    # 225: front parapet; 1830-1888 is a plain lower stretch in the artwork.
    p225 = behind_shed(pts(225))
    G.prism(M(225), p225, GROUND, 565.0)
    crenel_run(M(225), '225', [p225[4], p225[3], p225[2], p225[1], p225[0]], p225[6],
               [(1748, 1765), (1773, 1789), (1797, 1830), (1888, 1921)], 565.0, 575.0, 575.0,
               thickness=6.0)
    # 222/223: lower facade pieces behind the courtyard shed, from the courtyard.
    # Round 2: faces that touched or crossed the shed back wall line are clamped behind it.
    # 222 was a native sloped-top volume (358 at the shed roof line rising to 542) whose top
    # received facade artwork as a fake roof.  It is now a vertical plinth with a flat top at
    # the room floor (470), matching the 223 plinth beside it; the facade above belongs to 225.
    G.prism(M(222), behind_facade(pts(222)), GROUND, 470.0)
    # 223 stood wholly proud of the facade; it becomes a 3 px plinth band just behind it.
    G.prism(M(223), [(1750.8, 1501.4), (1780.6, 1495.5), (1780.6, 1492.5), (1750.8, 1498.4)], GROUND, 472.0)
    # 330-333: Patch07 room furniture standing on the room floor (was from z = 0).
    for n in (330, 331, 332):
        G.prism(M(n), pts(n), 472.0, so[n]['points'][0]['z_top'])
    G.prism(M(333), pts(333), 472.0, [p['z_top'] for p in so[333]['points']])
    return out


# ============================================================ turrets

CONE = dict(cx=1945.0, cy=1452.0, r=35.0, bottom=392.0, tip=352.0, tip_r=12.0,
            eave=597.0, eave_r=43.0, apex=647.0, pole_top=682.0)
# Round 2: the turret centre moves 6 native y north and every level rises 6, which leaves
# every source pixel (x, y - z) unchanged but keeps the body behind the courtyard shed's
# back wall (the round-1 circle pierced the shed roof's north-west corner by ~10 units).
TURRET = dict(cx=1719.0, cy=1489.0, r=50.0, crown_in=42.5, floor=484.0, notch=498.0,
              top=509.0, merlons=11, phase=84.0, merlon_deg=14.0)

# Courtyard shed back-wall outer line (native, node 195 south edge); annex faces must
# stay at or behind it.  Positive margin keeps a small clearance.
SHED_BACK = ((1944.0, 1473.0), (1725.0, 1519.0))


# Front facade plane of parapet wall 225 (native), west and east of its jog at x 1822-1827.
FACADE = [((1751.0, 1502.0), (1822.0, 1488.0)), ((1826.8, 1496.7), (1921.5, 1477.6))]


def behind_facade(poly, margin=0.3):
    """Clamp plinth vertices so nothing stands proud of the 225 facade plane."""
    out = []
    for x, y in behind_shed(poly):
        seg = FACADE[0] if x <= 1824.4 else FACADE[1]
        (ax, ay), (bx, by) = seg
        limit = ay + (by - ay) * (x - ax) / (bx - ax) - margin
        out.append((x, min(y, limit)))
    return out


def behind_shed(poly, margin=0.3):
    (ax, ay), (bx, by) = SHED_BACK
    out = []
    for x, y in poly:
        if bx <= x <= ax:
            limit = ay + (by - ay) * (x - ax) / (bx - ax) - margin
            y = min(y, limit)
        out.append((x, y))
    return out


def cone_turret_spec():
    c = CONE
    out = {node(197): G.Mesh(), node(199): G.Mesh(), node(200): G.Mesh()}
    # Corbelled bartizan: the artwork shows it hanging well above the courtyard.
    G.cylinder(out[node(197)], c['cx'], c['cy'], c['r'], c['bottom'], c['eave'] + 3.0, n=40)
    G.frustum(out[node(197)], c['cx'], c['cy'], c['tip_r'], c['r'], c['tip'], c['bottom'] + 0.5, n=40)
    G.cone(out[node(199)], c['cx'], c['cy'], c['eave_r'], c['eave'], c['apex'], n=40, start=90, end=270)
    G.cone(out[node(200)], c['cx'], c['cy'], c['eave_r'], c['eave'], c['apex'], n=40, start=-90, end=90)
    G.cylinder(out[node(199)], c['cx'], c['cy'], 1.5, c['apex'] - 4.0, c['pole_top'], n=8)
    return out


def round_turret_spec():
    t = TURRET
    out = {node(206): G.Mesh(), node(228): G.Mesh()}
    G.cylinder(out[node(206)], t['cx'], t['cy'], t['r'], GROUND, t['floor'] + 2.0, n=48)
    G.crenellated_ring(out[node(206)], t['cx'], t['cy'], t['r'] + 0.5, t['crown_in'], t['floor'] - 2.0,
                       t['notch'], t['top'], t['merlons'], t['phase'], t['merlon_deg'])
    G.cylinder(out[node(228)], t['cx'], t['cy'], t['crown_in'] + 0.5, t['floor'] - 8.0, t['floor'], n=48)
    return out


# ========================================================= north hall

def north_hall_spec():
    so = native_obstacles()
    pts = lambda i: [(p['x'], p['y']) for p in so[i]['points']]
    out = {node(n): G.Mesh() for n in (182, 183, 184, 185)}
    EVIDENCE.clear()
    p183, p184, p185 = pts(183), pts(184), pts(185)
    footprint = [p184[3], p184[0], p185[1], p185[2]]
    G.prism(out[node(185)], footprint, GROUND, 361.0)
    # Front parapet: twelve measured merlons (x >= 1850), three inferred where the
    # keep round tower hides the west end.
    lefts = [1784, 1806, 1828, 1850, 1871.5, 1895, 1916, 1939, 1960, 1983.5, 2005, 2026, 2048, 2069]
    spans = [(x, x + 12) for x in lefts] + [(2088, 2097.5)]
    crenel_run(out[node(185)], '185', [p185[2], p185[1]], (1900.0, 980.0), spans, 361.0, 371.0, 371.0,
               thickness=6.0)
    EVIDENCE['185']['inferred_hidden_spans'] = [[x, x + 12] for x in lefts[:3]]
    # Gable roof: 183 south slope (visible), 184 north slope (hidden behind ridge).
    G.prism(out[node(183)], [p183[3], p183[0], p183[1], p183[2]], 355.0, [412.0, 412.0, 362.0, 362.0])
    G.prism(out[node(184)], [p184[3], p184[0], p184[1], p184[2]], 355.0, [370.0, 370.0, 412.0, 412.0])
    # 182: external stair from the wall walk (370) to the courtyard (220).
    p182 = pts(182)
    # Round 2: 18 steps.  Tread bands in covered.png repeat every ~11.8 px (rows 620, 632,
    # 644, 656, 667, 679) with the nose advancing ~4 px per step; 217 px total drop / 11.8.
    G.stair(out[node(182)], p182[0], p182[3], p182[1], p182[2], 370.0, GROUND, 18, GROUND)
    return out


# ====================================================== curtain walls

WEST_SPANS = (
    # inferred behind the keep round tower (continuing the measured pitch)
    [(x, x + 13) for x in (1679, 1698, 1717, 1736, 1755, 1774, 1793, 1812, 1831)]
    + [(1850, 1863), (1868, 1882), (1888, 1902)]
    # western half-round bastion
    + [(1922, 462, 1922, 450), (1923, 441, 1930, 436), (1936, 1948, 433), (1958, 1972, 430),
       (1980, 1993, 440)]
    + [(2002, 2015), (2022, 2035), (2043, 2057), (2065, 2077), (2085, 2098), (2107, 2118),
       (2127, 2138), (2148, 2160), (2168, 2180), (2190, 2202), (2210, 2223), (2232, 2243),
       (2253, 2265)]
    # eastern half-round bastion
    + [(2263, 346, 2268, 331), (2275, 2288, 318), (2297, 2310, 316), (2318, 2330, 328)])
EAST_SPANS = [(2333, 2346), (2355, 2370), (2378, 2392), (2400, 2415), (2422, 2436),
              (2444, 2459), (2465, 2480), (2487, 2502), (2510, 2525), (2532, 2547)]


def curtain_spec(walk, parapet, spans, inferred):
    so = native_obstacles()
    pts = lambda i: [(p['x'], p['y']) for p in so[i]['points']]
    out = {node(walk): G.Mesh(), node(parapet): G.Mesh()}
    EVIDENCE.clear()
    G.prism(out[node(walk)], pts(walk), GROUND, 370.0)
    band = pts(parapet)
    G.prism(out[node(parapet)], band, 366.0, 386.0)
    ztop = max(p['z_top'] for p in so[parapet]['points'])
    if parapet == 187:
        path = list(reversed(band[:11]))
        inside = (2000.0, 850.0)
    else:
        path = [band[2], band[1]]
        inside = (2450.0, 780.0)
    crenel_run(out[node(parapet)], str(parapet), path, inside, spans, 386.0, ztop, ztop, thickness=6.0)
    EVIDENCE[str(parapet)]['inferred_hidden_spans'] = inferred
    return out


SPECS = {'lincoln-keep': keep_spec,
         'lincoln-keep-annex': annex_spec,
         'lincoln-keep-annex-cone-turret': cone_turret_spec,
         'lincoln-keep-annex-round-turret': round_turret_spec,
         'lincoln-north-hall': north_hall_spec,
         'lincoln-north-curtain-wall-west': lambda: curtain_spec(181, 187, WEST_SPANS, [list(s) for s in WEST_SPANS[:9]]),
         'lincoln-north-curtain-wall-east': lambda: curtain_spec(180, 186, EAST_SPANS, [list(EAST_SPANS[-1])])}
PREVIEW_BOX = {'lincoln-keep': (1432, 40, 1889, 1100),
               'lincoln-keep-annex': (1590, 640, 2000, 1300),
               'lincoln-keep-annex-cone-turret': (1880, 740, 2000, 1130),
               'lincoln-keep-annex-round-turret': (1640, 980, 1790, 1300),
               'lincoln-north-hall': (1680, 440, 2150, 800),
               'lincoln-north-curtain-wall-west': (1650, 280, 2350, 780),
               'lincoln-north-curtain-wall-east': (2300, 290, 2580, 620)}


def native_obstacles():
    return json.loads((REFINE / 'source-states/level.json').read_text())['sight_obstacles']



# ======================================================= mask revisions
# Reviewed receiver revisions for this lane's own nodes, applied to the
# workspace's working source-masks.json from the frozen mask-reference
# assignments on every run (idempotent).  Evidence images live in each
# workspace's inspection/ directory.
KEEP_NODES = [190, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 221, 226, 227,
              326, 327, 328, 329, 348, 459, 460]
MASK_REVISIONS = {
    'lincoln-keep': {'nodes': KEEP_NODES, 'drop_exclude': [158], 'add_exclude': [], 'add_include': [],
        'reason': 'mask158 contains the right-hand stonework of the keep round tower (x 1790-1858); the bush '
                  'it outlines stands east of the tower, not in front of it.'},
    'lincoln-north-curtain-wall-west': {'nodes': [181, 187], 'drop_exclude': [159, 49, 158, 48],
        'add_exclude': [], 'add_include': [243],
        'reason': 'mask159 (x 1980-2190), mask49 (x 2200-2420), mask158 (x 1787-2063) and mask48 (x 2131-2340) '
                  'are native envelopes that contain the visible walk, parapet and merlons; their foliage lies '
                  'north of (behind) the wall.  mask78/85 (the tree in front of the wall face) and mask245 '
                  '(north hall) stay excluded.  mask244 ends at x 2251, so the eastern half-round bastion '
                  '(x 2260-2336) is drawn inside the east-run envelope mask243, which is added as an include; '
                  'first-hit gating keeps east-run pixels on the east-run geometry.'},
    'lincoln-north-curtain-wall-east': {'nodes': [180, 186], 'drop_exclude': [49], 'add_exclude': [],
        'add_include': [],
        'reason': 'mask49 contains the parapet and merlons between x 2200-2420 (stonework; its foliage lies '
                  'behind the wall).'},
    # Round 2 (user: "why there a red bar across? some texture missing"): native roof mask399
    # ends at y 941 and body mask224 starts at y 947, leaving a 6 px unaccepted band and edge
    # slivers.  The annex envelope mask255 contains the whole turret silhouette; first-hit
    # gating keeps the annex merlon in front on the annex.
    'lincoln-keep-annex-cone-turret': {'nodes': [197, 199, 200], 'drop_exclude': [], 'add_exclude': [],
        'add_include': [255],
        'reason': 'mask399 (roof) and mask224 (body) leave a 6 px gap at y 941-947 across the turret; the annex '
                  'envelope mask255 contains the complete turret silhouette and is added as an include '
                  '(evidence inspection/mask-revision-cone-masks.png).'},
    # Round 2 (user: "stairs should be separate and they are missing texture"): the stair has
    # its own native silhouette mask247, which no receiver claimed; mask245 stops at the hall end.
    'lincoln-north-hall': {'nodes': [182], 'drop_exclude': [], 'add_exclude': [78, 85], 'add_include': [247],
        'set_include': [247],
        'reason': 'native mask247 is the stair silhouette (x 2092-2130, y 610-700) and was unassigned; mask245 '
                  '(hall) does not cover the stair, so the stair receiver now uses mask247 only.  The foreground '
                  'tree masks 78/85 are excluded where foliage hangs over the upper steps (evidence '
                  'inspection/mask-revision-stair-247.png).'},
    # mask413 was reviewed for the round turret and rejected: its envelope also covers the
    # turret's left masonry column (inspection/mask-revision-rock413.png), so no revision.
}


def revise_masks(asset):
    rev = MASK_REVISIONS.get(asset)
    ws = ASSETS / asset
    frozen = json.loads((ws / 'mask-reference/assignments.json').read_text())
    working = json.loads((ws / 'source-masks.json').read_text())
    base = {r['source_node']: r for r in frozen['projections']['exterior']['assignments']}
    changed = []
    for r in working['projections']['exterior']['assignments']:
        orig = base[r['source_node']]
        if not rev or r['source_node'] not in {node(n) for n in rev['nodes']}:
            continue
        r.clear()
        r.update(json.loads(json.dumps(orig)))
        base_inc = rev.get('set_include', orig['mask_indices'])
        r['mask_indices'] = list(base_inc) + [i for i in rev['add_include'] if i not in base_inc]
        excl = [i for i in orig.get('exclude_mask_indices', []) if i not in rev['drop_exclude']]
        excl += [i for i in rev['add_exclude'] if i not in excl]
        r['exclude_mask_indices'] = excl
        r['exclusions_reviewed'] = True
        r['exclusion_reason'] = (orig.get('exclusion_reason', '') + ' keep_north worker revision (round-1): '
                                 + rev['reason'] + ' Evidence: inspection/mask-revision.json.').strip()
        changed.append({'source_node': r['source_node'], 'mask_indices': r['mask_indices'],
                        'exclude_mask_indices': excl,
                        'frozen_mask_indices': orig['mask_indices'],
                        'frozen_exclude_mask_indices': orig.get('exclude_mask_indices', [])})
    (ws / 'source-masks.json').write_text(json.dumps(working, indent=2) + '\n')
    if rev:
        (ws / 'inspection/mask-revision.json').write_text(json.dumps(
            {'version': 1, 'asset_id': asset, 'reason': rev['reason'], 'changes': changed,
             'evidence_images': sorted(str(p) for p in (ws / 'inspection').glob('mask-revision-*.png'))},
            indent=1) + '\n')
    return changed

# ============================================================== Blender

def apply(asset, report_path):
    import bpy
    spec = SPECS[asset]()
    objs = [o for o in bpy.data.collections['lincoln Working'].all_objects
            if o.type == 'MESH' and o.get('asset_group') == asset]
    by_node = {}
    for o in objs:
        by_node.setdefault(o.get('source_node'), []).append(o)
    changes = []
    for n, mesh in sorted(spec.items()):
        if len(by_node.get(n, [])) != 1:
            raise ValueError(f'Expected one working mesh for {n}, found {len(by_node.get(n, []))}')
        changes.append({'source_node': n, **G.replace_mesh(by_node[n][0], mesh)})
    missing = sorted(set(by_node) - set(spec))
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=bpy.data.filepath)
    report = {'version': 1, 'asset_id': asset, 'recipe': str(Path(__file__).resolve()),
              'recipe_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'geometry_helper_sha256': hashlib.sha256((HERE / 'keep_north_geom.py').read_bytes()).hexdigest(),
              'ground_native_z': GROUND, 'changed': changes, 'unchanged_nodes': missing}
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + '\n')
    write_evidence(asset)
    return report


def write_evidence(asset):
    if not EVIDENCE:
        return
    src = REFINE / 'source-states/covered.png'
    record = {'version': 1, 'asset_id': asset, 'source': str(src),
              'source_sha256': hashlib.sha256(src.read_bytes()).hexdigest(),
              'method': 'Merlon shoulders measured on enlarged gridded crops of the original covered '
                        'artwork (and brightness profiles for the 201 run); corners are located on the '
                        'native parapet face projected at corner_projection_z, blocks span the parapet.',
              'confidence': 'manual 1-3 source pixels; inferred_hidden_spans are not source-measured',
              'runs': EVIDENCE}
    path = ASSETS / asset / 'inspection/crenel-corners.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(record, indent=1) + '\n')


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else sys.argv[1:]
    ap = argparse.ArgumentParser()
    ap.add_argument('--asset', required=True)
    ap.add_argument('--packet', action='store_true')
    ap.add_argument('--preview', action='store_true')
    ap.add_argument('--assets-dir', default=None)
    ap.add_argument('--no-mask-revision', action='store_true',
                    help='round 2: masks v2 already hold the reviewed revisions')
    args = ap.parse_args(argv)
    global ASSETS
    if args.assets_dir:
        ASSETS = Path(args.assets_dir).resolve()
    ws = ASSETS / args.asset
    if args.preview:
        import keep_north_preview as P
        spec = SPECS[args.asset]()
        write_evidence(args.asset)
        print(P.render(spec, PREVIEW_BOX[args.asset], ws / 'inspection/spec-preview.png'))
        return
    import bpy
    from render_slots import acquire
    acquire()
    if not args.no_mask_revision:
        revise_masks(args.asset)
    bpy.ops.wm.open_mainfile(filepath=str(ws / 'model.blend'))
    report = apply(args.asset, ws / 'inspection/geometry-recipe.json')
    print(json.dumps({k: report[k] for k in ('asset_id', 'unchanged_nodes')}))
    bad = [c for c in report['changed'] if c['nonmanifold_edges'] or c['degenerate_faces']]
    if bad:
        print('WARNING nonmanifold/degenerate:', json.dumps(bad))
    if args.packet:
        sys.path.insert(0, str(TOOLING))
        import refinement_workspace
        print(json.dumps(refinement_workspace.modified(str(ws)), default=str)[:2000])


if __name__ == '__main__':
    main()
