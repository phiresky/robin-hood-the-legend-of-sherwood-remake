"""Numbered battlement-corner traces for the Lincoln south gate/wall lane.

Offline (system Python with numpy + PIL). For every crenellated run the front
face is rectified from the frozen covered artwork: column s follows the native
front polyline, row z samples covered.png at (x, y - z). A vertical parapet is
undistorted in that elevation, so merlon shoulders and notch floors can be read
directly. Notch columns are classified from the luminance of a probe band
between the notch floor and merlon top (the walkway or courtyard seen through a
notch differs from the merlon face); every automatic interval is then written
as numbered corners, drawn on an annotated crop beside the untouched crop, and
inspected. Manual overrides are listed per run with a reason.

Usage: python3 south_gate_walls_trace.py <asset-id> [...]  (or 'all')
Writes <workspace>/inspection/corner-trace.json and corner-trace-<run>.png.
"""
import os
import hashlib
import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
from south_gate_walls_geometry import polyline_length, point_at  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
SOURCE = R / 'source-states/covered.png'
ASSETS = R / os.environ.get('SOUTH_GATE_ROUND', 'round-1') / 'assets'

# Each run: native outer (camera-facing) polyline and matching inner polyline,
# the z window of the elevation, probe band, and classification sense.
# 'notch_is' = 'bright' when a notch shows the lit walk/platform through it,
# 'dark' when it shows a shadowed background. Heights are read from the
# rectified elevation (grid every 10 native units) and apply to the run.
RUNS = {
    'lincoln-south-curtain-wall-central': [
        {'run': 'central-front', 'node': 'building-110',
         'outer': [(1344, 2060), (1777, 2014)], 'inner': [(1344, 2057), (1782, 2011)],
         'z': (300, 372), 'merlon_top': 355, 'sill': 345, 'probe': (348, 353),
         'notch_is': 'bright', 'min_width': 4,
         'notes': 'Round 2: merlon top lowered 358 -> 355. The silhouette top of each merlon in native mask '
                  '230 (358 measured at the front face) is the BACK edge of the 3-unit-deep cap, so the cap '
                  'surface is at 358 - 3 = 355; at 358 the caps overlapped 1-3 px of the south-wall cottage '
                  'roof drawn behind them (courtyard-lane cross-check, round-2 coverage-hits).'},
        {'run': 'central-bastion', 'node': 'building-110',
         'outer': [(1777, 2014), (1805, 2024), (1838, 2020), (1850, 2006), (1846, 1992)],
         'inner': [(1782, 2011), (1804, 2019), (1833, 2015), (1843, 2004), (1840, 1988)],
         'z': (300, 380), 'merlon_top': 364, 'sill': 351, 'probe': (354, 361),
         'notch_is': 'bright', 'min_width': 3, 'confidence': 'measured-manual',
         'manual_x': [(1784, 1790), (1805, 1811), (1827, 1834)],
         'notes': 'Stair treads behind the half-round bastion defeat automatic classification; '
                  'notches read manually from a 6x gridded crop (1-2 px uncertainty). The returning '
                  'east side is in deep shadow; no notch is asserted there.'},
    ],
    'lincoln-south-curtain-wall-west': [
        {'run': 'west-a', 'node': 'building-376',
         'outer': [(756, 1808), (861, 1885)], 'inner': [(762, 1806), (868, 1887)],
         'z': (340, 395), 'merlon_top': 379, 'sill': 366, 'probe': (369, 376),
         'notch_is': 'bright', 'min_width': 4},
        {'run': 'west-bump', 'node': 'building-376',
         'outer': [(861, 1885), (843, 1894), (882, 1924), (899, 1915)],
         'inner': [(868, 1887), (852, 1895), (883, 1918), (899, 1911)],
         'z': (340, 400), 'merlon_top': 382, 'sill': 368, 'probe': (371, 378),
         'notch_is': 'bright', 'min_width': 2, 'drop': [(0, 2.5), (70, 90)],
         'notes': 'Dropped: a 3 px sliver at the west return and the lit corner pilaster at the east corner '
                  '(x 881-897), which is solid stone in the crop, not a notch. Round 2 (user: "middle part should be sticking out and also have battlements"): the projecting '
                  'middle bay (16 units proud of the curtain face) carries its own merlons, 3-4 units higher than '
                  'the curtain run.'},
        {'run': 'west-c', 'node': 'building-376',
         'outer': [(899, 1915), (993, 1988)], 'inner': [(899, 1911), (999, 1985)],
         'z': (340, 395), 'merlon_top': 379, 'sill': 366, 'probe': (369, 376),
         'notch_is': 'bright', 'min_width': 4},
    ],
    'lincoln-south-gatehouse-arch': [
        {'run': 'gate-front', 'node': 'building-092/113/089',
         'outer': [(1070, 2066), (1094, 2074), (1167, 2094), (1212, 2107)],
         'inner': [(1071, 2062), (1095, 2070), (1168, 2090), (1213, 2103)],
         'z': (400, 465), 'merlon_top': 450, 'sill': 437, 'probe': (438, 441),
         'notch_is': 'bright', 'min_width': 3, 'confidence': 'measured-manual',
         'manual_x': [(1082, 1093), (1108, 1119), (1133, 1144), (1158, 1169), (1183, 1194)],
         'notes': 'Automatic pass found the same five notches but split/narrowed the third (1133-1136) '
                  'and added a 3 px sliver at the east-tower junction (1209-1212). Widths set from the '
                  'regular 25 px merlon rhythm visible in the elevation strip.'},
        {'run': 'gate-back', 'node': 'building-115/116/114',
         'outer': [(1107, 1998), (1146, 2009), (1196, 2021), (1256, 2038)],
         'inner': [(1109, 1995), (1149, 2005), (1198, 2018), (1259, 2035)],
         'z': (395, 445), 'merlon_top': 430, 'sill': 417, 'probe': (421, 427),
         'notch_is': 'unmasked', 'masks': [248, 270], 'min_width': 3},
    ],
    'lincoln-south-gatehouse-west-tower': [
        {'run': 'wt-front', 'node': 'building-121',
         'outer': [(965, 2012), (982, 2034), (1016, 2049), (1065, 2049)],
         'inner': [(973, 2014), (981, 2030), (1019, 2045), (1070, 2044)],
         'z': (395, 445), 'merlon_top': 432, 'sill': 419, 'probe': (422, 429),
         'notch_is': 'bright', 'min_width': 3},
        {'run': 'wt-back', 'node': 'building-121',
         'outer': [(973, 2014), (976, 1996), (985, 1987), (1001, 1976), (1044, 1971), (1082, 1978), (1106, 2000)],
         'inner': [(965, 2012), (972, 1994), (984, 1982), (1001, 1972), (1044, 1966), (1092, 1977), (1110, 1994)],
         'z': (395, 445), 'merlon_top': 432, 'sill': 419, 'probe': (422, 429),
         'notch_is': 'unmasked', 'masks': [248, 270], 'min_width': 3},
    ],
    'lincoln-south-gatehouse-east-tower': [
        {'run': 'et-front', 'node': 'building-111',
         'outer': [(1206, 2088), (1217, 2106), (1264, 2118), (1295, 2117), (1320, 2112), (1342, 2095), (1352, 2076)],
         'inner': [(1211, 2086), (1221, 2101), (1254, 2113), (1280, 2114), (1319, 2107), (1342, 2087), (1343, 2065)],
         'z': (395, 445), 'merlon_top': 432, 'sill': 419, 'probe': (422, 429),
         'notch_is': 'bright', 'min_width': 3, 'confidence': 'measured-manual',
         'manual_x': [(1230, 1236), (1252, 1264), (1280, 1290), (1306, 1315), (1329, 1334), (1342, 1348)],
         'notes': 'Merlon edges read from the 4x crop; the automatic pass matched four notches, merged '
                  '1320-1335 across a merlon and added slivers at 1210-1215/1226-1230 (gate-block junction '
                  'and antialiasing). The far (NE) parapet behind the cone turret has no reliable notch '
                  'evidence and stays solid.'},
    ],
    'lincoln-south-wall-stair': [
        {'run': 'diag', 'node': 'building-109',
         'outer': [(1845, 1990), (1930, 1915)], 'inner': [(1840, 1988), (1925, 1913)],
         'z': (300, 380), 'merlon_top': 362, 'sill': 348, 'probe': (351, 358),
         'notch_is': 'dark', 'min_width': 2, 'drop': [(95, 120)],
         'notes': 'This SE-facing parapet is lit; notch openings read as dark slots between lit merlon sides. '
                  'The dark interval at x 1918-1930 is the cone-turret shadow/junction, not a notch (dropped).'},
    ],
    'lincoln-southeast-curtain-wall': [
        {'run': 'se-front', 'node': 'building-099',
         'outer': [(1994, 1883), (2329, 1843)], 'inner': [(1991, 1881), (2330, 1840)],
         'z': (340, 395), 'merlon_top': 380, 'sill': 367, 'probe': (370, 377),
         'notch_is': 'bright', 'min_width': 4},
        {'run': 'se-turret', 'node': 'building-099',
         'outer': [(2329, 1843), (2343, 1850), (2369, 1850), (2390, 1844), (2396, 1814), (2387, 1810)],
         'inner': [(2330, 1840), (2343, 1847), (2367, 1847), (2387, 1840), (2393, 1818), (2378, 1811)],
         'z': (340, 395), 'merlon_top': 380, 'sill': 367, 'probe': (370, 377),
         'notch_is': 'bright', 'min_width': 3, 'confidence': 'measured-manual',
         'manual_x': [(2333, 2341), (2356, 2366), (2381, 2386)],
         'notes': 'Automatic classification found these three front notches plus two spurious '
                  'intervals where the ring turns north into shadow (x 2391-2396); those are not notches '
                  'in the crop and are omitted. East-facing merlons of the ring are hidden in shadow.'},
    ],
}


# Ground contact: the bottom edge of the reviewed native occluder mask along a
# camera-facing wall face gives the lowest visible masonry row per column;
# z = y_front - bottom_px. Bottoms above the plateau datum are clamped to it
# (the wall stands on the plateau there, grass/rock hides the footing).
GROUND = {
    'lincoln-south-curtain-wall-central': [
        {'profile': 'central-front', 'outer': [(1344, 2060), (1777, 2014)],
         'masks': [230], 'window': (1780, 1900), 'clamp': 218, 'step': 8},
    ],
    'lincoln-south-gatehouse-west-tower': [
        {'profile': 'wt-front', 'outer': [(976, 2027), (982, 2034), (1016, 2049), (1065, 2049)],
         'masks': [248], 'window': (1700, 1910), 'clamp': 218, 'step': 6},
    ],
    'lincoln-south-gatehouse-east-tower': [
        {'profile': 'et-front', 'outer': [(1217, 2106), (1264, 2118), (1295, 2117), (1320, 2112), (1342, 2095), (1352, 2076)],
         'masks': [248], 'window': (1700, 1930), 'clamp': 218, 'step': 6},
    ],
    'lincoln-south-wall-stair': [
        {'profile': 'diag-front', 'outer': [(1845, 1990), (1930, 1915)],
         'masks': [230, 231, 408], 'window': (1600, 1880), 'clamp': 218, 'step': 6},
        {'profile': 'bastion-front', 'outer': [(1778, 2014), (1805, 2024), (1838, 2020), (1850, 2006)],
         'masks': [230, 231, 408], 'window': (1600, 1880), 'clamp': 218, 'step': 4},
    ],
    'lincoln-south-wall-cone-turret': [
        {'profile': 'turret-junction', 'outer': [(1956, 1882), (1993, 1884)],
         'masks': [231], 'window': (1500, 1880), 'clamp': 218, 'step': 4},
    ],
    'lincoln-southeast-curtain-wall': [
        {'profile': 'se-front', 'outer': [(1994, 1883), (2329, 1843)],
         'masks': [231, 232], 'window': (1600, 1860), 'clamp': 218, 'step': 8},
    ],
    'lincoln-southeast-corner-turret': [
        {'profile': 'se-turret-front', 'outer': [(2329, 1843), (2343, 1850), (2369, 1850), (2390, 1844), (2396, 1814)],
         'masks': [231, 232], 'window': (1600, 1860), 'clamp': 218, 'step': 4},
        {'profile': 'se-east-face', 'outer': [(2396, 1814), (2480, 1677)],
         'masks': [232], 'window': (1290, 1860), 'clamp': 218, 'step': 8},
    ],
}
MASKS = R / 'mask-review/inventory-v1'


def native_mask(index):
    man = json.loads((MASKS / 'manifest.json').read_text())
    rec = [m for m in man['masks'] if m['index'] == index][0]
    a = np.array(Image.open(MASKS / rec['png'])) > 0
    full = np.zeros((2176, 2944), bool)
    x0, y0 = rec['box_top_left']
    h, w = a.shape
    full[y0:y0 + h, x0:x0 + w] = a[:2176 - y0, :2944 - x0]
    return full


def ground_profile(spec):
    m = np.zeros((2176, 2944), bool)
    for i in spec['masks']:
        m |= native_mask(i)
    L = polyline_length(spec['outer'])
    lo, hi = spec['window']
    rows = []
    n = max(2, int(math.ceil(L / spec['step'])) + 1)
    for k in range(n):
        s = min(L, k * L / (n - 1))
        x, y = point_at(spec['outer'], s)
        cols = [c for c in range(int(round(x)) - 2, int(round(x)) + 3)]
        bottoms = []
        for c in cols:
            ys = np.where(m[lo:hi, c])[0]
            if len(ys):
                bottoms.append(lo + ys.max())
        if not bottoms:
            continue  # column outside the mask (silhouette edge); neighbours define it
        b = int(max(bottoms))
        z = float(y - b)
        rows.append({'s': round(s, 2), 't': round(s / L, 5), 'x': round(x, 2), 'y_front': round(y, 2),
                     'bottom_px': int(b), 'z_art': round(z, 1), 'z': round(min(z, spec['clamp']), 1)})
    if len(rows) < n / 2:
        raise ValueError(f"Mask bottom missing for most of {spec['profile']}")
    return {'profile': spec['profile'], 'outer_polyline': spec['outer'], 'masks': spec['masks'],
            'clamp_z': spec['clamp'], 'samples': rows}


# Foreground vegetation masks drawn over owned receivers (reviewed exclusions).
FOREGROUND = {
    'lincoln-southeast-curtain-wall': [(153, (1950, 1420, 2090, 1540))],
    'lincoln-south-gatehouse-west-tower': [(66, (935, 1740, 1030, 1840))],
}


def foreground_evidence(asset, img, out_dir):
    rows = []
    for index, (x0, y0, x1, y1) in FOREGROUND.get(asset, []):
        m = native_mask(index)[y0:y1, x0:x1]
        crop = img[y0:y1, x0:x1].copy()
        edge = m & ~(np.roll(m, 1, 0) & np.roll(m, -1, 0) & np.roll(m, 1, 1) & np.roll(m, -1, 1))
        marked = crop.copy()
        marked[edge] = (0, 255, 0)
        s = 4
        a = Image.fromarray(crop).resize(((x1 - x0) * s, (y1 - y0) * s), Image.NEAREST)
        b = Image.fromarray(marked).resize(a.size, Image.NEAREST)
        sheet = Image.new('RGB', (a.width * 2 + 8, a.height), (20, 20, 20))
        sheet.paste(a, (0, 0))
        sheet.paste(b, (a.width + 8, 0))
        path = out_dir / f'foreground-mask-{index}.png'
        sheet.save(path)
        rows.append({'mask': index, 'box': [x0, y0, x1, y1], 'image': str(path), 'sha256': sha(path)})
    return rows


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load():
    return np.array(Image.open(SOURCE).convert('RGB'))


def samples(poly, step=0.5):
    L = polyline_length(poly)
    n = int(L / step)
    return [(i * step, point_at(poly, i * step)) for i in range(n + 1)], L


def rectify(img, poly, z0, z1, step=0.5):
    pts, L = samples(poly, step)
    H = z1 - z0
    out = np.zeros((H, len(pts), 3), np.uint8)
    for j, (_, (x, y)) in enumerate(pts):
        for i in range(H):
            z = z1 - i
            out[i, j] = img[int(round(y - z)), int(round(x))]
    return out, pts, L


def classify(img, run):
    z0, z1 = run['z']
    elev, pts, L = rectify(img, run['outer'], z0, z1)
    lum = elev.astype(float) @ np.array([0.3, 0.59, 0.11])
    p0, p1 = run['probe']
    if run['notch_is'] == 'unmasked':
        # Far parapets: a notch shows background that the reviewed native
        # occluder mask excludes, so the probe band leaves the mask there.
        m = np.zeros((2176, 2944), bool)
        for i in run['masks']:
            m |= native_mask(i)
        band = np.array([np.mean([m[int(round(y - z)), int(round(x))] for z in range(p0, p1 + 1)])
                         for _, (x, y) in pts])
        thr = 0.5
        notch = band < thr
    else:
        band = lum[z1 - p1:z1 - p0 + 1].mean(axis=0)
        thr = run.get('threshold') or (np.percentile(band, 15) + np.percentile(band, 85)) / 2
        notch = band > thr if run['notch_is'] == 'bright' else band < thr
    intervals, start = [], None
    for j, flag in enumerate(list(notch) + [False]):
        if flag and start is None:
            start = j
        elif not flag and start is not None:
            intervals.append((pts[start][0], pts[j - 1][0]))
            start = None
    step = pts[1][0] - pts[0][0]
    intervals = [(a, b + step) for a, b in intervals if b + step - a >= run.get('min_width', 3)]
    if 'manual_x' in run:
        # Manual reading from the gridded source crop; converted to arc length.
        from south_gate_walls_geometry import arc_param_at_x
        intervals = [tuple(sorted((arc_param_at_x(run['outer'], a), arc_param_at_x(run['outer'], b))))
                     for a, b in run['manual_x']]
    for a, b in run.get('drop', []):
        intervals = [iv for iv in intervals if not (a <= (iv[0] + iv[1]) / 2 <= b)]
    intervals += [tuple(iv) for iv in run.get('add', [])]
    return sorted(intervals), elev, pts, L, float(thr)


def trace_run(img, run):
    intervals, elev, pts, L, thr = classify(img, run)
    inner_L = polyline_length(run['inner'])
    top, sill = run['merlon_top'], run['sill']
    corners, notches = [], []
    for k, (s0, s1) in enumerate(intervals):
        t0, t1 = s0 / L, min(s1 / L, 1.0)
        a, b = point_at(run['outer'], s0), point_at(run['outer'], min(s1, L))
        notches.append({'index': k + 1, 's0': round(s0, 2), 's1': round(s1, 2),
                        't0': round(t0, 5), 't1': round(t1, 5),
                        'front_left': [round(a[0], 2), round(a[1], 2)],
                        'front_right': [round(b[0], 2), round(b[1], 2)]})
        for (x, y), z, role in ((a, top, 'left-shoulder-top'), (a, sill, 'left-notch-floor'),
                                (b, sill, 'right-notch-floor'), (b, top, 'right-shoulder-top')):
            corners.append({'id': len(corners) + 1, 'notch': k + 1, 'role': role,
                            'pixel': [round(x, 2), round(y - z, 2)],
                            'native': [round(x, 2), round(y, 2), z],
                            'edge_traced': 'outer (camera-facing) parapet face',
                            'confidence': run.get('confidence', 'measured'),
                            'visibility': 'visible'})
    return {'run': run['run'], 'source_node': run['node'],
            'outer_polyline': run['outer'], 'inner_polyline': run['inner'],
            'outer_length': round(L, 3), 'inner_length': round(inner_L, 3),
            'merlon_top_z': top, 'sill_z': sill, 'probe_band_z': list(run['probe']),
            'notch_is': run['notch_is'], 'threshold': round(thr, 2),
            'manual_drop': run.get('drop', []), 'manual_x': run.get('manual_x', []), 'manual_add': run.get('add', []),
            'notes': run.get('notes', ''), 'notches': notches, 'corners': corners}, elev


def annotate(img, record, elev, out):
    xs = [c['pixel'][0] for c in record['corners']] + [p[0] for p in record['outer_polyline']]
    ys = [c['pixel'][1] for c in record['corners']] + [p[1] - record['merlon_top_z'] for p in record['outer_polyline']]
    x0, x1 = int(min(xs)) - 12, int(max(xs)) + 12
    y0, y1 = int(min(ys)) - 20, int(max(ys)) + 30
    scale = 4 if (x1 - x0) < 260 else 3
    crop = Image.fromarray(img[y0:y1, x0:x1]).resize(((x1 - x0) * scale, (y1 - y0) * scale), Image.NEAREST)
    marked = crop.copy()
    d = ImageDraw.Draw(marked)
    pts = [((c['pixel'][0] - x0 + .5) * scale, (c['pixel'][1] - y0 + .5) * scale) for c in record['corners']]
    for k in range(0, len(pts), 4):
        d.line(pts[k:k + 4], fill=(0, 255, 255), width=1)
    for c, (X, Y) in zip(record['corners'], pts):
        col = (255, 255, 0) if 'top' in c['role'] else (255, 64, 255)
        d.ellipse([X - 2, Y - 2, X + 2, Y + 2], outline=col)
        if c['id'] % 2 == 1 or len(pts) < 40:
            d.text((X - 6, Y - 12 if 'top' in c['role'] else Y + 2), str(c['id']), fill=col)
    el = Image.fromarray(elev).resize((elev.shape[1] * 2, elev.shape[0] * 3), Image.NEAREST)
    de = ImageDraw.Draw(el)
    z1 = record['probe_band_z']
    for n in record['notches']:
        a, b = n['s0'] / 0.5 * 2, n['s1'] / 0.5 * 2
        de.rectangle([a, 0, b, 5], fill=(255, 0, 255))
    W = max(crop.width * 2 + 8, el.width)
    sheet = Image.new('RGB', (W, crop.height + el.height + 8), (20, 20, 20))
    sheet.paste(crop, (0, 0))
    sheet.paste(marked, (crop.width + 8, 0))
    sheet.paste(el, (0, crop.height + 8))
    sheet.save(out)
    return {'crop_origin': [x0, y0], 'crop_size': [x1 - x0, y1 - y0], 'scale': scale}


def trace_asset(asset, img):
    runs = RUNS.get(asset, [])
    out_dir = ASSETS / asset / 'inspection'
    out_dir.mkdir(parents=True, exist_ok=True)
    records = []
    for run in runs:
        record, elev = trace_run(img, run)
        png = out_dir / f"corner-trace-{run['run']}.png"
        record['annotation'] = annotate(img, record, elev, png)
        record['annotation']['image'] = str(png)
        record['annotation']['sha256'] = sha(png)
        records.append(record)
    doc = {'version': 1, 'asset_id': asset, 'method': __doc__.strip().splitlines()[0],
           'source': str(SOURCE), 'source_sha256': sha(SOURCE),
           'tool': str(Path(__file__).resolve()), 'tool_sha256': sha(__file__),
           'runs': records,
           'foreground_masks': foreground_evidence(asset, img, out_dir),
           'ground_profiles': [ground_profile(g) for g in GROUND.get(asset, [])]}
    (out_dir / 'corner-trace.json').write_text(json.dumps(doc, indent=1) + '\n')
    return doc


if __name__ == '__main__':
    img = load()
    targets = sorted(set(RUNS) | set(GROUND)) if sys.argv[1:] == ['all'] else sys.argv[1:]
    for asset in targets:
        doc = trace_asset(asset, img)
        for r in doc['runs']:
            print(asset, r['run'], 'notches', len(r['notches']), 'thr', r['threshold'])
