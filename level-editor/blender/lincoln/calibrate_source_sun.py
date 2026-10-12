"""Calibrate the Lincoln review sun direction from cast shadows in the source artwork.

Runs with system python3 + Pillow + numpy (no Blender):

    python3 level-editor/blender/lincoln/calibrate_source_sun.py

Writes into level-editor/work/needs-review/lincoln-refinement/lighting-calibration/:
  estimate.json, map-lighting.json and annotated evidence PNGs.

Projection: native (x, y, z) -> pixel (x, y - z).  Blender world X = x,
Y = -y / sin35, Z = z / cos35.  A world point P casts onto the horizontal plane
Z0 at P - (P.Z - Z0) / v.z * v, where v is the unit vector toward the sun.

Landmarks are exact caster/shadow pixels picked on nearest-neighbour enlarged,
gridded crops.  Casters are the two blue slate cone turrets whose finials and
cones throw crisp shadows onto flat paved receivers whose heights are known
from native sight obstacles:

  * keep-walkway turret (obstacles 197/199/200) over the keep entrance walkway
    (obstacle 198);
  * north-east corner turret (obstacles 162/164/165) over the NE tower platform
    (obstacles 160/163).
"""

import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / "level-editor/work/needs-review/lincoln-refinement"
OUT = R / "lighting-calibration"
SOURCE = R / "source-states/covered.png"
LEVEL = R / "source-states/level.json"

S = math.sin(math.radians(35))
C = math.cos(math.radians(35))
ENDPOINT_PX = 2.0  # +- bound applied to every picked endpoint


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def world(x, y, z):
    return np.array([x, -y / S, z / C], float)


def pix(p):
    p = np.asarray(p, float)
    return np.stack([p[..., 0], -p[..., 1] * S - p[..., 2] * C], -1)


def vec(az, el):
    a, e = math.radians(az), math.radians(el)
    return np.array([math.cos(e) * math.cos(a), math.cos(e) * math.sin(a), math.sin(e)])


def az_el(v):
    v = np.asarray(v, float) / np.linalg.norm(v)
    return math.degrees(math.atan2(v[1], v[0])), math.degrees(math.asin(v[2]))


def shadow(p, z0, v):
    p = np.asarray(p, float)
    return p - ((p[..., 2] - z0 / C) / v[2])[..., None] * v


def pair_direction(top, tip, height):
    """Toward-sun vector from caster top pixel, its shadow tip pixel and the
    caster height (native z units) above the receiver plane."""
    base_y = top[1] + height
    dx, dy = tip[0] - top[0], tip[1] - base_y
    v = np.array([-dx / height, dy / (height * S), 1 / C])
    return v / np.linalg.norm(v)


def line_azimuth(points):
    """Azimuth of the shadow of a vertical edge from points along that shadow line."""
    p = np.asarray(points, float)
    q = p - p.mean(0)
    _, _, vt = np.linalg.svd(q)
    d = vt[0]
    if d[0] > 0:
        d = -d  # shadows run toward -x in this map; orientation only matters for the sign
    # image direction of a vertical edge shadow is (-vx, vy*S)
    return math.degrees(math.atan2(d[1] / S, -d[0]))


# ---------------------------------------------------------------- polygons
def hull(pts):
    pts = sorted(map(tuple, np.asarray(pts, float)))

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lo, up = [], []
    for p in pts:
        while len(lo) >= 2 and cross(lo[-2], lo[-1], p) <= 0:
            lo.pop()
        lo.append(p)
    for p in reversed(pts):
        while len(up) >= 2 and cross(up[-2], up[-1], p) <= 0:
            up.pop()
        up.append(p)
    return np.array(lo[:-1] + up[:-1])


def inside(poly, pts):
    pts = np.atleast_2d(pts)
    a, b = poly, np.roll(poly, -1, 0)
    ay, by = a[:, 1][None], b[:, 1][None]
    px, py = pts[:, 0][:, None], pts[:, 1][:, None]
    with np.errstate(divide="ignore", invalid="ignore"):
        xint = a[:, 0][None] + (py - ay) * (b[:, 0] - a[:, 0])[None] / (by - ay)
    hit = ((ay > py) != (by > py)) & (px < xint)
    return hit.sum(1) % 2 == 1


def boundary_segments(polys):
    segs = []
    for i, poly in enumerate(polys):
        a, b = poly, np.roll(poly, -1, 0)
        keep = np.ones(len(a), bool)
        for j, other in enumerate(polys):
            if j != i:
                keep &= ~inside(other, (a + b) / 2)
        segs.append(np.stack([a[keep], b[keep]], 1))
    return np.concatenate(segs)


def point_seg_dist(pts, segs):
    pts = np.atleast_2d(np.asarray(pts, float))[:, None]
    a, b = segs[None, :, 0], segs[None, :, 1]
    ab = b - a
    t = np.clip(((pts - a) * ab).sum(-1) / np.maximum((ab * ab).sum(-1), 1e-9), 0, 1)
    return np.linalg.norm(pts - (a + t[..., None] * ab), axis=-1).min(1)


def turret_polys(t, v, n=72):
    th = np.linspace(0, 2 * math.pi, n, endpoint=False)

    def circ(r, z):
        return np.stack([world(t["x"] + r * math.cos(a), t["y"] + r * math.sin(a) * S, z) for a in th])

    cone = np.vstack([pix(shadow(world(t["x"], t["y"], t["apex_z"]), t["recv_z"], v))[None],
                      pix(shadow(circ(t["eave_r"], t["eave_z"]), t["recv_z"], v))])
    body = np.vstack([pix(circ(t["body_r"], t["recv_z"])),
                      pix(shadow(circ(t["body_r"], t["eave_z"]), t["recv_z"], v))])
    return [hull(cone), hull(body)]


# ---------------------------------------------------------------- geometry
def obstacle(level, i):
    return level["sight_obstacles"][i]["points"]


def geometry(level):
    """Turret/receiver geometry from native obstacles, cross-checked against the
    eave ellipse measured in the artwork (eave centre pixel y + eave z = axis y)."""
    t1_ring = obstacle(level, 199) + obstacle(level, 200)
    t1_y = (min(p["y"] for p in t1_ring) + max(p["y"] for p in t1_ring)) / 2
    walk = obstacle(level, 198)
    walk_z = (walk[0]["z_bottom"] + walk[0]["z_top"]) / 2
    ne_ring = obstacle(level, 162)
    ne_y = (min(p["y"] for p in ne_ring) + max(p["y"] for p in ne_ring)) / 2
    ne_floor = (obstacle(level, 160)[0]["z_bottom"] + obstacle(level, 163)[0]["z_top"]) / 2
    # Image measurements (pixels): eave ellipse centre y, eave half width, apex, body half width.
    t1 = dict(name="keep-walkway turret", x=1950.5, y=t1_y, eave_z=t1_y - 856.0, eave_r=41.5,
              apex_z=t1_y - 797.0, body_r=36.5, recv_z=walk_z)
    ne = dict(name="north-east turret", x=2834.0, y=ne_y, eave_z=ne_y - 367.3, eave_r=43.0,
              apex_z=ne_y - 297.0, body_r=37.0, recv_z=ne_floor)
    checks = {
        "keep_turret_eave_z_from_image": t1["eave_z"],
        "keep_turret_eave_z_obstacles": [min(p["z_top"] for p in t1_ring), max(p["z_bottom"] for p in t1_ring)],
        "ne_turret_eave_z_from_image": ne["eave_z"],
        "ne_turret_eave_z_obstacle_162_top": ne_ring[0]["z_top"],
        "ne_turret_apex_z_from_image": ne["apex_z"],
        "ne_turret_cone_top_obstacles_164_165": max(p["z_top"] for p in obstacle(level, 164)),
    }
    return t1, ne, checks


# ---------------------------------------------------------------- landmarks
BALL_TOP = (1952.0, 777.0)       # finial ball centre, keep-walkway turret
BALL_SHADOW = (1826.0, 866.5)    # ball shadow blob centre on the walkway
POLE_STRIP = [(1831, 869), (1833, 870), (1836, 871), (1838, 871), (1840, 873), (1842, 873),
              (1844, 874), (1846, 874), (1847, 875)]
T1_CONE_EDGE = [(1849, 878), (1853, 880), (1854, 882), (1856, 884), (1857, 886), (1859, 888),
                (1860.5, 890), (1862, 892), (1862, 894), (1862, 896), (1863, 898), (1863, 900),
                (1863, 902), (1864, 904)]
# Lit/shadow transition, 0.5 px above the first lit pixel of each column.
NE_EAVE_EDGE = [(2739, 400.5), (2742, 405.5), (2745, 409.5), (2748, 414.5), (2751, 417.5), (2754, 420.0)]
NE_BODY_EDGE = [(2757, 421.5), (2760, 423.5), (2763, 425.0), (2766, 425.5), (2769, 426.5), (2772, 427.5),
                (2775, 428.5), (2778, 428.5), (2781, 429.0), (2784, 429.5), (2787, 431.0), (2793, 433.5),
                (2796, 434.5), (2799, 435.5), (2802, 436.5)]


def residuals(az, el, t1, ne):
    v = vec(az, el)
    ball = world(BALL_TOP[0], t1["y"], t1["y"] - BALL_TOP[1])
    rb = float(np.linalg.norm(pix(shadow(ball, t1["recv_z"], v)) - BALL_SHADOW))
    apex = world(BALL_TOP[0], t1["y"], t1["apex_z"])
    seg = pix(shadow(np.stack([apex, ball]), t1["recv_z"], v))[None]
    rs = float(np.sqrt(np.mean(point_seg_dist(POLE_STRIP, seg) ** 2)))
    r1 = float(np.sqrt(np.mean(point_seg_dist(T1_CONE_EDGE, boundary_segments(turret_polys(t1, v))) ** 2)))
    nsegs = boundary_segments(turret_polys(ne, v))
    r2 = float(np.sqrt(np.mean(point_seg_dist(NE_EAVE_EDGE + NE_BODY_EDGE, nsegs) ** 2)))
    return dict(ball=rb, pole_strip=rs, keep_cone_edge=r1, ne_edges=r2)


def grid_fit(t1, ne, keys, az_range=(-50, -12), el_range=(36, 62), step=0.5):
    best = (1e18, None)
    for az in np.arange(az_range[0], az_range[1] + 1e-9, step):
        for el in np.arange(el_range[0], el_range[1] + 1e-9, step):
            r = residuals(az, el, t1, ne)
            cost = sum(r[k] ** 2 for k in keys)
            if cost < best[0]:
                best = (cost, (float(az), float(el)), r)
    return best


def refine(t1, ne, keys, start, span=1.5, step=0.1):
    return grid_fit(t1, ne, keys, (start[0] - span, start[0] + span), (start[1] - span, start[1] + span), step)


def main():
    global BALL_SHADOW, POLE_STRIP, T1_CONE_EDGE, NE_EAVE_EDGE, NE_BODY_EDGE
    level = json.loads(LEVEL.read_text())
    t1, ne, checks = geometry(level)
    src = Image.open(SOURCE).convert("RGB")

    # --- closed-form pair and azimuth-only lines
    h_ball = t1["y"] - t1["recv_z"] - BALL_TOP[1]
    v_ball = pair_direction(BALL_TOP, BALL_SHADOW, h_ball)
    corners = []
    for ex in (-1, 1):
        for ey in (-1, 1):
            for tx in (-1, 1):
                for dh in (-7, 7):
                    top = (BALL_TOP[0] + tx * 1.0, BALL_TOP[1])
                    tip = (BALL_SHADOW[0] + ex * ENDPOINT_PX, BALL_SHADOW[1] + ey * ENDPOINT_PX)
                    corners.append(az_el(pair_direction(top, tip, h_ball + dh)))
    az_strip = line_azimuth(POLE_STRIP)
    az_ne_line = line_azimuth(NE_BODY_EDGE)

    # --- model fits, one per independent structure and joint
    fits = {}
    for name, keys in [("keep_turret_ball_pair", ["ball"]),
                       ("keep_turret_pole_and_cone_edges", ["pole_strip", "keep_cone_edge"]),
                       ("ne_turret_edges", ["ne_edges"]),
                       ("joint", ["ball", "pole_strip", "keep_cone_edge", "ne_edges"])]:
        coarse = grid_fit(t1, ne, keys, step=1.0)
        fine = refine(t1, ne, keys, coarse[1])
        fits[name] = dict(azimuth_degrees=fine[1][0], elevation_degrees=fine[1][1],
                          rms_px={k: round(fine[2][k], 3) for k in keys})

    # --- bounds: joint refit under geometry perturbations (receiver z, axis y) and
    #     every endpoint shifted by +-ENDPOINT_PX as a rigid bias in x or y.
    base_marks = (BALL_SHADOW, POLE_STRIP, T1_CONE_EDGE, NE_EAVE_EDGE, NE_BODY_EDGE)
    keys = ["ball", "pole_strip", "keep_cone_edge", "ne_edges"]
    joint = fits["joint"]
    envelope = [(joint["azimuth_degrees"], joint["elevation_degrees"])]
    perturb = []
    for dz in (-4, 4):
        perturb.append(("receiver_z %+d" % dz, dict(t1, recv_z=t1["recv_z"] + dz), dict(ne, recv_z=ne["recv_z"] + dz), (0, 0)))
    for dy in (-5, 5):
        perturb.append(("axis_y %+d (eave z tied to image)" % dy,
                        dict(t1, y=t1["y"] + dy, eave_z=t1["eave_z"] + dy, apex_z=t1["apex_z"] + dy),
                        dict(ne, y=ne["y"] + dy, eave_z=ne["eave_z"] + dy, apex_z=ne["apex_z"] + dy), (0, 0)))
    for d in ((ENDPOINT_PX, 0), (-ENDPOINT_PX, 0), (0, ENDPOINT_PX), (0, -ENDPOINT_PX)):
        perturb.append(("endpoints shifted %+g,%+g px" % d, t1, ne, d))
    bound_runs = []
    for label, a, b, d in perturb:
        sh = lambda pts: [(x + d[0], y + d[1]) for x, y in pts]
        BALL_SHADOW = (base_marks[0][0] + d[0], base_marks[0][1] + d[1])
        POLE_STRIP, T1_CONE_EDGE, NE_EAVE_EDGE, NE_BODY_EDGE = map(sh, base_marks[1:])
        r = refine(a, b, keys, (joint["azimuth_degrees"], joint["elevation_degrees"]), span=4, step=0.25)
        envelope.append(r[1])
        bound_runs.append(dict(perturbation=label, azimuth_degrees=r[1][0], elevation_degrees=r[1][1]))
    BALL_SHADOW, POLE_STRIP, T1_CONE_EDGE, NE_EAVE_EDGE, NE_BODY_EDGE = base_marks
    # structure disagreement also counts toward the bound
    for k in ("keep_turret_ball_pair", "keep_turret_pole_and_cone_edges", "ne_turret_edges"):
        envelope.append((fits[k]["azimuth_degrees"], fits[k]["elevation_degrees"]))
    az_b = [min(e[0] for e in envelope), max(e[0] for e in envelope)]
    el_b = [min(e[1] for e in envelope), max(e[1] for e in envelope)]

    az, el = joint["azimuth_degrees"], joint["elevation_degrees"]
    toward = vec(az, el).tolist()

    # --- low-confidence check: archery targets (not used in the fit)
    targets = []
    for n, (top, ground_z) in enumerate([((2116, 1154, 247), 220), ((2212, 1152, 243), 220),
                                         ((2310, 1160, 243), 220), ((2383, 1178, 247), 220)], 1):
        p = world(*top)
        targets.append(dict(target=n, top_native=top, receiver_z=ground_z,
                            predicted_tip_px=[round(float(c), 1) for c in pix(shadow(p, ground_z, vec(az, el)))]))

    landmarks = [
        dict(id=1, kind="point pair", structure="keep-walkway turret finial ball -> paved walkway",
             caster_px=list(BALL_TOP), shadow_px=list(BALL_SHADOW), caster_height_native=round(h_ball, 2),
             base_px=[BALL_TOP[0], round(BALL_TOP[1] + h_ball, 2)],
             toward_sun=v_ball.tolist(), azimuth_degrees=az_el(v_ball)[0], elevation_degrees=az_el(v_ball)[1],
             endpoint_bounds={"azimuth": [min(c[0] for c in corners), max(c[0] for c in corners)],
                              "elevation": [min(c[1] for c in corners), max(c[1] for c in corners)],
                              "assumes": "shadow +-%g px, caster x +-1 px, caster height +-7 native units" % ENDPOINT_PX},
             confidence="high for pixels (distinct ball highlight and isolated round blob on flat lit paving); "
                        "medium for height, which comes from obstacle 199/200 ring centre and obstacle 198 walkway z",
             why="Only compact, isolated caster feature in the map whose shadow falls on a flat, lit, "
                 "height-known receiver; ball blob sits exactly on the continuation of the pole shadow."),
        dict(id=2, kind="vertical-edge shadow line (azimuth only)", structure="keep-walkway turret finial pole -> walkway",
             caster_px=[[1952, 797], [1952, 777]], shadow_px=POLE_STRIP, azimuth_degrees=az_strip,
             confidence="high: 1-2 px wide crisp strip, darkest-pixel trace per column",
             why="Shadow of a vertical pole on a horizontal plane runs along the sun azimuth regardless of receiver height."),
        dict(id=3, kind="silhouette edge (model fit)", structure="keep-walkway turret cone -> walkway",
             caster="cone eave ellipse centre (1950.5, 856) half-width 41.5 px, apex (1951, 797)",
             shadow_px=T1_CONE_EDGE, confidence="medium: soft 1-2 px edge; lower rows near the front parapet excluded",
             why="Lower-left boundary of the cone shadow; compared to the projected hull of apex+eave circle shadow."),
        dict(id=4, kind="vertical-edge shadow line (azimuth only)", structure="north-east turret cylinder -> NE tower platform",
             caster="tangent silhouette of turret body, half-width 37 px", shadow_px=NE_BODY_EDGE, azimuth_degrees=az_ne_line,
             confidence="high: straight 45 px lit/shade transition on flat paving",
             why="Tangent of a vertical cylinder casts a straight line along the azimuth; independent turret and receiver."),
        dict(id=5, kind="silhouette edge (model fit)", structure="north-east turret cone eave -> NE tower platform",
             caster="cone eave ellipse centre (2834, 367.3) half-width 43 px, apex (2838, 297)",
             shadow_px=NE_EAVE_EDGE, confidence="medium: curved edge meets the north parapet at its upper end",
             why="Curved part of the same shadow boundary; constrains elevation independently of landmark 1."),
    ]

    report = dict(
        status="SOURCE-CALIBRATED-REVIEW-ESTIMATE",
        map="lincoln",
        source="level-editor/work/needs-review/lincoln-refinement/source-states/covered.png",
        source_sha256=sha(SOURCE),
        level_sha256=sha(LEVEL),
        projection="native (x,y,z) -> pixel (x, y-z); world X=x, Y=-y/sin35, Z=z/cos35",
        azimuth_convention="degrees, atan2(toward_sun.y, toward_sun.x) in Blender world; -90 = due south (toward the camera)",
        geometry=dict(keep_turret=t1, ne_turret=ne, cross_checks=checks),
        landmarks=landmarks,
        per_structure_fits=fits,
        toward_sun=toward,
        azimuth_degrees=az,
        elevation_degrees=el,
        uncertainty={"azimuth": az_b, "elevation": el_b,
                     "method": "envelope of joint refits under receiver z +-4, axis y +-5, rigid endpoint shifts "
                               "+-%g px, plus per-structure fits" % ENDPOINT_PX},
        bound_runs=bound_runs,
        hypothesis_check=dict(
            coordinator_hypothesis="sun roughly south-east and fairly low",
            result="partly refuted: sun is east-south-east (about 30 deg south of east), elevation about 49 deg, not low. "
                   "Long left-pointing blobs of the archery targets come from their leaning shape and soft contact "
                   "darkening, not from a low sun.",
            directional_vs_point="Two independent turrets ~900 px apart fit the same direction within the bounds "
                                 "(see per_structure_fits); consistent with a directional sun. A distant point light "
                                 "cannot be excluded at this precision; no evidence requires one."),
        low_confidence_checks=dict(archery_targets=targets,
                                   note="Targets (obstacles 170-177, courtyard z 220) not used: rounded leaning bales, "
                                        "soft shadow blobs merged with contact darkening; obstacle heights coarse."),
        exclusions=[
            "Archery targets: soft blobs, leaning rounded casters, unknown top correspondence.",
            "South-courtyard fence posts, cart, stable posts: shadows lost in wheel ruts and painted mud streaks.",
            "Well: roof shadow merges with rut texture.",
            "Keep roof platform lit patch and SW bastion shadow boundary: casters are compound (parapets, tower, stair turret); "
            "correspondence ambiguous.",
            "Wall-walk merlon teeth (east wall, keep walkway parapet): crisp but merlon heights are below obstacle precision.",
            "Village cottages: bases hidden under eaves, overlapping roof shadows on thatch.",
            "Foliage/tree shadows, rock faces, river reflections, ambient-occlusion contact darkening and painted facade "
            "shading excluded throughout.",
        ],
        limitations=[
            "Two independent structures (keep-walkway turret, NE turret), both slate cone turrets; five landmarks total.",
            "Caster heights rely on native sight-obstacle z (coarse) cross-checked against measured eave ellipses (agree within ~3 units).",
            "Finial pole above the ball casts no visible shadow; the ball blob is used as the tip.",
            "Directional sun assumed; a distant point light is not excluded.",
            "Not validated against a Blender render of the refined Lincoln geometry.",
        ],
        production_defaults_changed=False,
    )
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "estimate.json").write_text(json.dumps(report, indent=2) + "\n")
    est_sha = sha(OUT / "estimate.json")
    (OUT / "map-lighting.json").write_text(json.dumps(dict(
        version=1, map="lincoln", status="artwork-calibrated-review-estimate",
        lighting=dict(toward_sun=toward, ambient=0.16, diffuse=0.64, shadow_epsilon=0.03),
        azimuth_degrees=az, elevation_degrees=el,
        uncertainty={"azimuth": az_b, "elevation": el_b},
        estimate_sha256=est_sha, source_sha256=report["source_sha256"],
        limitations=report["limitations"],
        review="Landmarks annotated in evidence-keep-turret.png, evidence-ne-turret.png and evidence-overview.png; "
               "awaiting coordinator inspection. Use for review renders only; production lighting unchanged.",
    ), indent=2) + "\n")

    annotate(src, t1, ne, vec(az, el), h_ball, targets)
    return report


def annotate(src, t1, ne, v, h_ball, targets):
    def crop(box, scale):
        im = src.crop(box).resize(((box[2] - box[0]) * scale, (box[3] - box[1]) * scale), Image.NEAREST)
        return im, ImageDraw.Draw(im), (lambda p: ((p[0] - box[0]) * scale, (p[1] - box[1]) * scale))

    def outline(draw, to, polys, color):
        for seg in boundary_segments(polys):
            draw.line([to(seg[0]), to(seg[1])], fill=color, width=1)

    def mark(draw, xy, label, color, r=5):
        x, y = xy
        draw.ellipse((x - r, y - r, x + r, y + r), outline=color, width=2)
        draw.text((x + r + 2, y - r), label, fill=color, stroke_width=2, stroke_fill="black")

    # keep-walkway turret
    box, sc = (1790, 740, 1995, 935), 5
    im, d, to = crop(box, sc)
    outline(d, to, turret_polys(t1, v), (255, 255, 0))
    base = (BALL_TOP[0], BALL_TOP[1] + h_ball)
    d.line([to(BALL_TOP), to(BALL_SHADOW)], fill="cyan", width=2)
    d.line([to(base), to(BALL_SHADOW)], fill="cyan", width=1)
    mark(d, to(BALL_TOP), "1C ball", "cyan")
    mark(d, to(BALL_SHADOW), "1S", "cyan")
    mark(d, to(base), "1B base (walkway z)", "cyan", 3)
    for p in POLE_STRIP:
        mark(d, to(p), "", "magenta", 3)
    d.text(to((1828, 876)), "2 pole shadow", fill="magenta", stroke_width=2, stroke_fill="black")
    d.line([to((1952, 797)), to((1952, 777))], fill="magenta", width=3)
    for p in T1_CONE_EDGE:
        mark(d, to(p), "", "lime", 3)
    d.text(to((1840, 905)), "3 cone edge", fill="lime", stroke_width=2, stroke_fill="black")
    d.text((8, 8), "Keep-walkway turret: 1 ball pair, 2 pole-shadow line, 3 cone edge; yellow = predicted shadow at estimate",
           fill="white", stroke_width=2, stroke_fill="black")
    im.save(OUT / "evidence-keep-turret.png")

    # north-east turret
    box, sc = (2690, 270, 2890, 470), 5
    im, d, to = crop(box, sc)
    outline(d, to, turret_polys(ne, v), (255, 255, 0))
    for p in NE_BODY_EDGE:
        mark(d, to(p), "", "cyan", 3)
    for p in NE_EAVE_EDGE:
        mark(d, to(p), "", "lime", 3)
    base = (ne["x"], ne["y"] - ne["recv_z"])
    apex = (2838, 297)
    tip = pix(shadow(world(ne["x"], ne["y"], ne["apex_z"]), ne["recv_z"], v))
    d.line([to(apex), to(tip)], fill="orange", width=1)
    mark(d, to(apex), "apex", "orange", 3)
    mark(d, to(tip), "apex shadow (predicted, behind parapet)", "orange", 3)
    mark(d, to(base), "axis base", "cyan", 3)
    d.text(to((2770, 440)), "4 body tangent line", fill="cyan", stroke_width=2, stroke_fill="black")
    d.text(to((2700, 412)), "5 eave edge", fill="lime", stroke_width=2, stroke_fill="black")
    d.text((8, 8), "North-east turret: 4 body tangent (azimuth), 5 eave edge; yellow = predicted shadow at estimate",
           fill="white", stroke_width=2, stroke_fill="black")
    im.save(OUT / "evidence-ne-turret.png")

    # archery targets (excluded, predicted tips shown for reference only)
    box, sc = (2050, 880, 2420, 990), 3
    im, d, to = crop(box, sc)
    for t in targets:
        mark(d, to(t["predicted_tip_px"]), "T%d tip?" % t["target"], "red", 4)
    d.text((8, 8), "EXCLUDED archery targets: red = predicted top-shadow tip at estimate (soft leaning blobs, not used)",
           fill="white", stroke_width=2, stroke_fill="black")
    im.save(OUT / "evidence-targets-excluded.png")

    # overview
    ov = src.resize((src.width // 2, src.height // 2), Image.BOX)
    d = ImageDraw.Draw(ov)
    for box, label, color in [((1790, 740, 1995, 935), "L1-3 keep-walkway turret", "cyan"),
                              ((2690, 270, 2890, 470), "L4-5 NE turret", "cyan"),
                              ((2050, 880, 2420, 990), "excluded: targets", "red"),
                              ((1640, 1280, 1960, 1480), "excluded: fence/cart", "red"),
                              ((1640, 1530, 1780, 1630), "excluded: well", "red"),
                              ((280, 1330, 520, 1560), "excluded: bastion", "red"),
                              ((1470, 200, 1640, 320), "excluded: keep roof", "red"),
                              ((0, 150, 760, 700), "excluded: village", "red")]:
        b = [c // 2 for c in box]
        d.rectangle(b, outline=color, width=3)
        d.text((b[0] + 4, b[1] + 4), label, fill=color, stroke_width=2, stroke_fill="black")
    ov.save(OUT / "evidence-overview.png")


if __name__ == "__main__":
    rep = main()
    print(json.dumps(dict(toward_sun=rep["toward_sun"], azimuth=rep["azimuth_degrees"], elevation=rep["elevation_degrees"],
                          uncertainty=rep["uncertainty"], fits=rep["per_structure_fits"],
                          pair1=[rep["landmarks"][0][k] for k in ("azimuth_degrees", "elevation_degrees", "endpoint_bounds")],
                          strip_az=rep["landmarks"][1]["azimuth_degrees"], ne_line_az=rep["landmarks"][3]["azimuth_degrees"]),
                     indent=1))
