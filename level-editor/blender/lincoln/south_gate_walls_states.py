"""Drawbridge state check for the Lincoln south gate (offline, numpy + PIL).

Projects the recipe's raised leaf (covered state, obstacle 457 active) onto
covered.png and the hinge-rotated lowered leaf onto the Pont_levis applied
frame (last transition frame composited over covered.png), side by side with
the untouched crops. Writes <drawbridge workspace>/inspection/drawbridge-states.png
and drawbridge-states.json (pixel outlines, overshoot measurement, hashes).
"""
import os
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import south_gate_walls_assets as A  # noqa: E402

ROOT = HERE.parents[2]
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
WS = R / os.environ.get('SOUTH_GATE_ROUND', 'round-1') / 'assets/lincoln-south-gate-drawbridge'
BOX = (1000, 1740, 1240, 2000)
SCALE = 3


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def lowered_frame():
    layers = json.loads((R / 'source-states/layers.json').read_text())
    patch = [p for p in layers['patches'] if p['id'] == 'patch-003'][0]
    frame = patch['states']['transition']['frames'][-1]
    img = Image.open(R / 'source-states/covered.png').convert('RGBA')
    sprite = Image.open(R / 'source-states' / frame['image']).convert('RGBA')
    img.alpha_composite(sprite, (frame['bbox'][0], frame['bbox'][1]))
    return img.convert('RGB'), frame


def leaf_corners(angle):
    x0, x1 = A.LEAF_X
    face = A.gate_line(x0) + 0.5 + A.LEAF_THICK, A.gate_line(x1) + 0.5 + A.LEAF_THICK
    pts = [(x0, face[0], A.LEAF_BOTTOM), (x1, face[1], A.LEAF_BOTTOM),
           (x1, face[1], A.LEAF_TOP), (x0, face[0], A.LEAF_TOP)]
    return A.rotate_native(pts, angle) if angle else pts


def draw(img, polys, colors):
    x0, y0, x1, y1 = BOX
    crop = img.crop(BOX).resize(((x1 - x0) * SCALE, (y1 - y0) * SCALE), Image.NEAREST)
    marked = crop.copy()
    d = ImageDraw.Draw(marked)
    for poly, col in zip(polys, colors):
        pts = [((x - x0 + .5) * SCALE, (y - y0 + .5) * SCALE) for x, y in poly]
        d.line(pts + pts[:1], fill=col, width=2)
    return crop, marked


def main():
    covered = Image.open(R / 'source-states/covered.png').convert('RGB')
    lowered, frame = lowered_frame()
    angle = A.lowered_angle()
    raised = [(x, y - z) for x, y, z in leaf_corners(0)]
    low = [(x, y - z) for x, y, z in leaf_corners(angle)]
    deck = [(x, y - 220.0) for x, y in [(1053, 2108), (1128, 2128), (1078, 2188), (1004, 2167)]]
    landing = [(x, y - 220.0) for x, y in [(1099, 2066), (1164, 2084), (1126, 2128), (1060, 2110)]]
    c1, m1 = draw(covered, [raised, deck], [(0, 255, 255), (255, 255, 0)])
    c2, m2 = draw(lowered, [low, landing, deck], [(255, 0, 255), (0, 255, 0), (255, 255, 0)])
    W, H = c1.size
    sheet = Image.new('RGB', (W * 4 + 24, H + 24), (20, 20, 20))
    for i, im in enumerate((c1, m1, c2, m2)):
        sheet.paste(im, (i * (W + 8), 24))
    d = ImageDraw.Draw(sheet)
    d.text((4, 4), 'covered | raised leaf 457 (cyan), footbridge 058 deck (yellow)', fill=(255, 255, 255))
    d.text((2 * (W + 8) + 4, 4), f'Pont_levis applied | lowered leaf ({angle:+.0f} deg, magenta), '
           'landing 059 (green)', fill=(255, 255, 255))
    out = WS / 'inspection/drawbridge-states.png'
    out.parent.mkdir(exist_ok=True)
    sheet.save(out)
    # Art lowered deck far edge (px) versus model far edge.
    art_far = [(1062, 1888), (1127, 1903)]
    report = {'version': 1, 'asset_id': 'lincoln-south-gate-drawbridge',
              'tool': str(Path(__file__).resolve()), 'tool_sha256': sha(__file__),
              'covered_sha256': sha(R / 'source-states/covered.png'),
              'lowered_frame': frame['image'], 'lowered_frame_sha256': frame['sha256'],
              'hinge_angle_deg': angle,
              'raised_leaf_px': [[round(a, 1) for a in p] for p in raised],
              'raised_mask415_extent_px': {'x': [1095, 1166], 'top': [1766, 1782], 'bottom': [1850, 1869]},
              'lowered_leaf_px': [[round(a, 1) for a in p] for p in low],
              'art_lowered_far_edge_px': art_far,
              'lowered_far_edge_overshoot_px': round(float(np.mean([low[2][1] - art_far[1][1], low[3][1] - art_far[0][1]])), 1),
              'image': str(out), 'image_sha256': sha(out)}
    (WS / 'inspection/drawbridge-states.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({k: report[k] for k in ('hinge_angle_deg', 'lowered_far_edge_overshoot_px')}))


if __name__ == '__main__' and '--deck' not in sys.argv:
    main()


# Authored footbridge deck domain for mask inventory v3 (covered.png pixels).
# Read on a 5x gridded crop: plank field between the two rail masks 179/180,
# from the gate landing edge to the drawn plank ends on the south bank. The
# diagonal brace under the east rail is listed separately (drawn in front of
# the ravine rock, belongs to 058 as structure).
DECK_DOMAIN_PX = [(1053, 1887), (1113, 1904), (1072, 1974), (1064, 1982), (995, 1967), (989, 1955), (1003, 1945)]
BRACE_DOMAIN_PX = [(1122, 1908), (1134, 1914), (1074, 1986), (1063, 1980)]


def deck_domain():
    img = Image.open(R / 'source-states/covered.png').convert('RGB')
    x0, y0, x1, y1 = 980, 1860, 1150, 2005
    s = 5
    crop = img.crop((x0, y0, x1, y1)).resize(((x1 - x0) * s, (y1 - y0) * s), Image.NEAREST)
    marked = crop.copy()
    d = ImageDraw.Draw(marked)
    for poly, col in ((DECK_DOMAIN_PX, (255, 255, 0)), (BRACE_DOMAIN_PX, (0, 255, 255))):
        pts = [((x - x0 + .5) * s, (y - y0 + .5) * s) for x, y in poly]
        d.line(pts + pts[:1], fill=col, width=2)
    sheet = Image.new('RGB', (crop.width * 2 + 8, crop.height), (20, 20, 20))
    sheet.paste(crop, (0, 0))
    sheet.paste(marked, (crop.width + 8, 0))
    out = WS / 'inspection/footbridge-deck-domain.png'
    sheet.save(out)
    doc = {'version': 1, 'asset_id': 'lincoln-south-gate-drawbridge', 'source_node': 'building-058',
           'purpose': 'Authored receiver domain for the footbridge deck top, which has no native occluder mask '
                      '(actors walk on it). Proposed for mask inventory v3 as an authored (non-native) mask; '
                      'rails already use native masks 179/180.',
           'source': str(R / 'source-states/covered.png'), 'source_sha256': sha(R / 'source-states/covered.png'),
           'state': 'covered (raised drawbridge); the same deck is drawn unchanged in the Pont_levis applied frame',
           'deck_polygon_px': DECK_DOMAIN_PX, 'brace_polygon_px': BRACE_DOMAIN_PX,
           'model_deck_top_native': {'z': 220.0, 'footprint': [[1053, 2108], [1128, 2128], [1074, 2199], [997, 2184]]},
           'uncertainty_px': 3, 'evidence': str(out), 'evidence_sha256': sha(out),
           'tool': str(Path(__file__).resolve())}
    (WS / 'inspection/footbridge-deck-domain.json').write_text(json.dumps(doc, indent=1) + '\n')
    return doc


if __name__ == '__main__' and '--deck' in sys.argv:
    print(deck_domain()['evidence'])
