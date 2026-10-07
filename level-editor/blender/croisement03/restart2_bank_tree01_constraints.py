"""Bounded CPU source constraints for the full bank and adjoining Tree01."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

R = Path(__file__).resolve().parents[3]
B = R / 'level-editor/work/croisement03-refinement'
O = B / 'restart2/northwest-bank-tree01-constraints-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def nearest_edge(obstacle, point):
    points = obstacle['points']
    result = []
    for i, a in enumerate(points):
        b = points[(i + 1) % len(points)]
        start = np.array([a['x'], a['y'] - a['z_top']])
        end = np.array([b['x'], b['y'] - b['z_top']])
        delta = end - start
        t = float(np.clip(np.dot(np.array(point) - start, delta) / np.dot(delta, delta), 0, 1))
        p = start + t * delta
        result.append(dict(edge=i, distance=float(np.linalg.norm(np.array(point) - p)),
            projected_point=p.tolist(), authored_height=a['z_top'] + t * (b['z_top'] - a['z_top'])))
    return min(result, key=lambda row: row['distance'])


def main():
    O.mkdir(exist_ok=False)
    source = B / 'baseline/covered.png'
    level_path = B / 'baseline/Croisement03.rhp.json'
    level = json.loads(level_path.read_text())
    src = Image.open(source).convert('RGB')
    constraints = []
    seen = set()
    for line in level['elevation_lines']:
        sides = (line['right_obstacle_index'], line['left_obstacle_index'])
        if not set(sides).intersection((52, 53, 54)):
            continue
        key = (tuple(line['point_a']), tuple(line['point_b']), sides)
        if key in seen:
            continue
        seen.add(key)
        row = dict(line=line, endpoints=[])
        for point in (line['point_a'], line['point_b']):
            surfaces = {str(i): nearest_edge(level['sight_obstacles'][i], point)
                for i in sides if i != 65535}
            row['endpoints'].append(dict(native_point=point, surfaces=surfaces))
        constraints.append(row)
    crop = (0, 0, 230, 390)
    marked = src.copy()
    draw = ImageDraw.Draw(marked)
    for i, color in [(2, '#ffff55'), (3, '#ff9944'), (4, '#55ffff'), (5, '#ff66cc')]:
        points = level['sight_obstacles'][i]['points']
        for side, width in [('z_top', 2), ('z_bottom', 1)]:
            poly = [(p['x'], p['y'] - p[side]) for p in points]
            draw.line(poly + [poly[0]], fill=color, width=width)
        draw.text((points[0]['x'], points[0]['y'] - points[0]['z_top']), str(i), fill=color)
    sheet = Image.new('RGB', (920, 810), '#222222')
    d = ImageDraw.Draw(sheet)
    for i, (label, im) in enumerate([('Native source: roots partly obscured', src), ('Native obstacle top / bottom outlines; not bark ownership', marked)]):
        sheet.paste(im.crop(crop).resize((460, 780), Image.Resampling.NEAREST), (i * 460, 30))
        d.text((i * 460 + 4, 8), label, fill='white')
    sheet.save(O / 'tree01-native-and-obstacles.png')
    mask = level['masks'][1]
    pins = [source, level_path, B/'restart2/geometry-round15-tree02-shared-ridge-v1/freeze.json',
        B/'restart2/tree02-ridge-ray-guard-v4/receipt.json']
    record = dict(status='CPU constraints only; source ownership and construction remain pending',
        source_hashes={str(p.relative_to(R)): sha(p) for p in pins}, bank_elevation_interfaces=constraints,
        added_dependency=dict(obstacle=94, reason='West ramp53 explicitly joins obstacle94; this separate variable-height surface must be retained in later joint review.'),
        tree01=dict(native_obstacles=[2, 3, 4, 5], native_mask=1,
            mask_box={k: mask[k] for k in ['box_top_left', 'box_size']},
            parts={str(i):level['sight_obstacles'][i] for i in [2, 3, 4, 5]},
            visual_findings=['The source shows separated visible stems; preserve their distinct paths rather than connecting all four coarse parts into an invented single fork.',
                'Low roots are obscured by shrubs and stone. The visible last bark pixel is not a measured ground elevation.',
                'The tall right stem touches the bank silhouette in projection. Its actual source rays must be checked with the full bank and west ramp.',
                'Canopy ownership is not inferred from the wood mask or nearby crown color; independent source partition is required.']),
        bank_requirements=['Preserve the hash-bound shared lip x175–340 exactly in the next bank candidate.',
            'Plateau52 and sloped surfaces53/54 are separate authored height profiles. A vertical box cannot replace their joins.',
            'Projected elevation-line endpoints have rounded integer coordinates; nearest-edge distances below are measurements, not permission to snap gameplay obstacles.',
            'Visible ivy/shrubs share the bank occlusion masks. A bank mask alone must not label them as stone texture.'],
        limits=['No model/render/API/live mutation.', 'No runtime ordering or animation proof.', 'No geometry or appearance approval.'])
    (O/'constraints.json').write_text(json.dumps(record, indent=2)+'\n')
    assert sum(p.stat().st_size for p in O.iterdir()) < 2 * 1024**2
    print(O/'constraints.json')


if __name__ == '__main__':
    main()
