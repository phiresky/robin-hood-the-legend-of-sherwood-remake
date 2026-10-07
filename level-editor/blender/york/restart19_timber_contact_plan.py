"""CPU-only loose-board contact hypothesis; no Blender or library changes.

Run with uv run --offline --with shapely --with pillow python <this file>.
"""
import hashlib
import json
import math
from pathlib import Path
from PIL import Image, ImageDraw
from shapely.geometry import MultiPoint, Polygon

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement/restart2'
OUT = WORK / 'timber-contact-plan-v1'
if OUT.exists():
    raise FileExistsError(OUT)
observations_path = WORK / 'timber-props-source-corners-v2/observations.json'
support_path = WORK / 'timber-props-source-study-v1/support-preflight-v1.json'
observations = json.loads(observations_path.read_text())
support = json.loads(support_path.read_text())
assert hashlib.sha256(observations_path.read_bytes()).hexdigest() == '55a0468795e422bd5779d4664b18c7bf416cd79e26eb4b3e2fc83c1fefa8528a'
prop = next(p for p in support['props'] if p['source_node'] == 'building-008')
hits = [p['ground_hit'] for p in prop['bottom_vertex_samples']]
assert all(p is not None for p in hits)
ground_z = sum(p[2] for p in hits) / len(hits)
ground_envelope = MultiPoint([(p[0], p[1]) for p in hits]).convex_hull
s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
marks = {r['id']: r['pixel'] for r in observations['corners']}

def world(pixel, z):
    return [pixel[0], -(pixel[1] + c * z) / s]

def projected(xy, z):
    return [xy[0], -s * xy[1] - c * z]

pieces = []
def piece(name, top_pixels, bottom, top, constraints):
    xy = [world(p, top) for p in top_pixels]
    poly = Polygon(xy)
    assert poly.is_valid and poly.area > 0
    pieces.append({'id': name, 'top_source_polygon': top_pixels,
                   'footprint_world': xy, 'bottom_z': bottom, 'top_z': top,
                   'constraints': constraints, 'polygon': poly})

# Hidden far ends are explicit continuations, not additional measured corners.
for name, a, b, shift in [('pale-1', 'A', 'B', (25, -16)),
                          ('pale-2', 'C', 'D', (25, -16)),
                          ('pale-3', 'E', 'F', (26, -17))]:
    p, q = marks[a], marks[b]
    far_q = [q[0]+shift[0], q[1]+shift[1]]
    far_p = [p[0]+shift[0], p[1]+shift[1]]
    piece(name, [p, q, far_q, far_p], ground_z, ground_z+6,
          {'measured_end_markers': [a, b], 'far_end': 'inferred behind crossing pieces',
           'thickness': 'inferred six world units'})
piece('pale-4', [marks['G'], marks['H'], marks['J'], marks['I']], ground_z, ground_z+6,
      {'measured_end_markers': ['G', 'H', 'I', 'J'],
       'shape': 'tapered hypothesis, manual two-pixel uncertainty', 'thickness': 'inferred'})
piece('long-crossing', [marks['K'], marks['L'], marks['M'], [1290, 956]],
      ground_z+6, ground_z+11,
      {'measured_markers': ['K', 'L', 'M'], 'fourth_top_corner': 'inferred above N side-face marker',
       'thickness': 'inferred five world units'})
# O/P describe exposed endpoints; solve a narrow complete beam around that axis.
top = ground_z+11
a, b = world(marks['O'], top), world(marks['P'], top)
dx, dy = b[0]-a[0], b[1]-a[1]
length = math.hypot(dx, dy)
nx, ny = -dy/length*2, dx/length*2
corners = [[a[0]+nx, a[1]+ny], [b[0]+nx, b[1]+ny],
           [b[0]-nx, b[1]-ny], [a[0]-nx, a[1]-ny]]
piece('short-crossing', [projected(p, top) for p in corners], ground_z+6, top,
      {'axis_markers': ['O', 'P'], 'width': 'inferred four world units',
       'thickness': 'inferred five world units'})

contacts, interpenetrations = [], []
for i, a in enumerate(pieces):
    for b in pieces[i+1:]:
        overlap = a['polygon'].intersection(b['polygon'])
        if overlap.area <= 1e-8:
            continue
        vertical = min(a['top_z'], b['top_z']) - max(a['bottom_z'], b['bottom_z'])
        if vertical > 1e-6:
            interpenetrations.append({'a': a['id'], 'b': b['id'],
                                     'area': overlap.area, 'depth': vertical})
        elif abs(a['top_z']-b['bottom_z']) < 1e-6 or abs(b['top_z']-a['bottom_z']) < 1e-6:
            lower, upper = (a, b) if a['top_z'] <= b['bottom_z'] else (b, a)
            contacts.append({'lower': lower['id'], 'upper': upper['id'],
                             'area': overlap.area, 'centroid_world': list(overlap.centroid.coords)[0]})

for p in pieces:
    p['diagnostic_outside_baseline_ground_sample_hull_area'] = p['polygon'].difference(ground_envelope).area
    p['supports'] = ['assumed-ground-plane'] if p['bottom_z'] == ground_z else [
        r['lower'] for r in contacts if r['upper'] == p['id']]
    del p['polygon']

OUT.mkdir()
report = {'status': 'PRIVATE_CPU_CONTACT_HYPOTHESIS_NOT_MODEL_READY',
          'source_observations_sha256': hashlib.sha256(observations_path.read_bytes()).hexdigest(),
          'support_evidence_sha256': hashlib.sha256(support_path.read_bytes()).hexdigest(),
          'ground_plane_z': ground_z, 'ground_sample_z_spread': max(p[2] for p in hits)-min(p[2] for p in hits),
          'pieces': pieces, 'bearing_contacts': contacts, 'interpenetrations': interpenetrations,
          'unsupported_pieces': [p['id'] for p in pieces if not p['supports']],
          'limits': ['No actual receiver triangle query for proposed feet yet.',
                     'Baseline sample hull is not the receiver boundary; outside area is diagnostic only.',
                     'No projection ownership, visual, material or eight-view review yet.',
                     'Six piece count is a visible-minimum hypothesis, not proven buried structure.',
                     'Planar contact area does not prove mechanical stability or natural construction.']}
(OUT/'plan.json').write_text(json.dumps(report, indent=2)+'\n')
crop_box = observations['crop']; scale = 10
image = Image.open(observations['source']).convert('RGB').crop(tuple(crop_box))
image = image.resize((image.width*scale, image.height*scale), Image.Resampling.NEAREST)
draw = ImageDraw.Draw(image)
colors = ['#ff5050', '#ffff00', '#30ffff', '#50ff50', '#f080ff', '#ffffff']
for p, color in zip(pieces, colors):
    points = [((x-crop_box[0]+.5)*scale, (y-crop_box[1]+.5)*scale) for x,y in p['top_source_polygon']]
    draw.line(points+[points[0]], fill=color, width=2)
    x, y = points[0]
    draw.text((x+3,y+3), p['id'], fill=color, stroke_width=2, stroke_fill='black')
image.save(OUT/'top-face-hypothesis.png')
print(json.dumps({'output':str(OUT),'contacts':len(contacts),'interpenetrations':interpenetrations,
                  'unsupported':report['unsupported_pieces']}))
