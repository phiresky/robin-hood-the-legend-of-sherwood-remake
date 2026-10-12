"""Detail pass for lincoln-hall-keep-north-wall (nodes 229, 231, 232).

The two parapets of the hall-to-keep wall walk are rebuilt as continuous notched walls
with real merlons. Merlon shoulders and heights come from the numbered source trace
``inspection/north-wall-merlon-trace.json`` (original covered.png pixels):

- North parapet 231: 9 visible merlons (left shoulders x 1290..1460). Least-squares
  period is 21.33 px and width 13.3 px. Two more merlons under the west-wing roof are
  inferred from that period.
- South parapet 232: 3 visible merlon west ends (x 1331, 1353, 1374), same period. The
  rest are hidden by the west-wing roof, the middle-tower chimney 259 and the east-wing
  roof, so they are inferred from the same period and phase.
- North run: the traced left-shoulder tops and notch floors are the *far* (north, back)
  cap edge of the silhouette. A merlon's top face is seen from above, so its upper
  silhouette is the back edge. These corners are converted with the back-face line and the
  front-face left shoulder is shifted by the wall's x offset (+4.74). The traced right
  shoulder x is the front-right corner, which is the silhouette's rightmost point. The
  native mask 252 upper boundary independently confirms this: step-ups at 1289, 1310,
  1332, ... and a ~3 px drop over the last ~4.7 px of every merlon cap.
- South run: the three traced points are front-face corners, taken at the right edge of
  each shadowed west end face, and are converted with the front-face line.
- Merlon top and notch floor z are per run: the mean of that run's measured corners.

The walk 229 (top z 550) keeps its native shape. The recipe has already cut its pillar
to the 420 ground. The parapet bodies run from the same 420 ground.
"""
from pathlib import Path

import great_hall_geom as G

TRACE = (Path(__file__).resolve().parents[2] /
         'work/needs-review/lincoln-refinement/round-1/assets/lincoln-hall-keep-north-wall/inspection/'
         'north-wall-merlon-trace.json')
GROUND = 420.0
PERIOD = 21.33


def _run(trace, run_id):
    for run in trace['runs']:
        if run['id'] == run_id:
            return run
    raise ValueError(f'Trace run missing: {run_id}')


def _merlons_from_lefts(p0, p1, lefts, top):
    out = []
    for left, right in lefts:
        ta = G.pixel_to_tz(p0, p1, left, 0)[0]
        tb = G.pixel_to_tz(p0, p1, right, 0)[0]
        if tb <= 0.0 or ta >= 1.0:
            continue
        out.append((max(ta, 0.0), min(tb, 1.0), top))
    return out


def _z(run, corner, edge):
    a, b = run['wall'][edge]
    return G.pixel_to_tz(a, b, corner['px'], corner['py'])[1]


def apply(by_node):
    trace = G.load_trace(TRACE)
    north, south = _run(trace, 'north-parapet'), _run(trace, 'south-parapet')
    result = {}

    # North parapet 231 (traced on the far cap edge -> back-face line).
    p0, p1 = north['wall']['front']
    q0, q1 = north['wall']['back']
    dx = p0[0] - q0[0]
    lefts = [c for c in north['corners'] if c['role'] == 'merlon-left-shoulder-top']
    rights = [c for c in north['corners'] if c['role'] == 'merlon-right-shoulder-top']
    floors = [c for c in north['corners'] if c['role'] == 'notch-floor']
    top = sum(_z(north, c, 'back') for c in lefts) / len(lefts)
    floor_z = sum(_z(north, c, 'back') for c in floors) / len(floors)
    measured = [(a['px'] + dx, b['px']) for a, b in zip(lefts, rights)]
    width = sum(b - a for a, b in measured) / len(measured)
    first = measured[0][0]
    inferred = [(first - k * PERIOD, first - k * PERIOD + width) for k in (2, 1)]
    merlons = _merlons_from_lefts(p0, p1, inferred + measured, top)
    profile = G.crenel_profile(GROUND, lambda t: floor_z, merlons)
    verts, faces = G.notched_wall(p0, p1, q0, q1, profile)
    stats = G.replace_mesh(by_node['building-231'], verts, faces)
    result['building-231'] = {**stats, 'merlons': len(merlons), 'measured_merlons': len(measured),
                              'inferred_merlons': len(inferred), 'merlon_top_z': round(top, 2),
                              'notch_floor_z': round(floor_z, 2), 'front_merlon_width_px': round(width, 2)}

    # South parapet 232 (front-face corners); same period and front width as the north run.
    p0, p1 = south['wall']['front']
    q0, q1 = south['wall']['back']
    tops = [c for c in south['corners'] if 'top' in c['role']]
    bottoms = [c for c in south['corners'] if 'floor' in c['role']]
    s_top = sum(_z(south, c, 'front') for c in tops) / len(tops)
    s_floor = sum(_z(south, c, 'front') for c in bottoms) / len(bottoms)
    seen = [c['px'] for c in tops]
    phase = sum(x - k * PERIOD for k, x in enumerate(seen)) / len(seen)
    lefts = []
    k = -3
    while phase + k * PERIOD <= p1[0]:
        left = phase + k * PERIOD
        if left + width > p0[0]:
            lefts.append((left, left + width))
        k += 1
    merlons = _merlons_from_lefts(p0, p1, lefts, s_top)
    profile = G.crenel_profile(GROUND, lambda t: s_floor, merlons)
    verts, faces = G.notched_wall(p0, p1, q0, q1, profile)
    stats = G.replace_mesh(by_node['building-232'], verts, faces)
    result['building-232'] = {**stats, 'merlons': len(merlons), 'measured_merlons': len(seen),
                              'inferred_merlons': len(merlons) - len(seen), 'merlon_top_z': round(s_top, 2),
                              'notch_floor_z': round(s_floor, 2), 'front_merlon_width_px': round(width, 2)}
    return result
