"""Build closed provisional Tree18 sweeps without assigning source materials."""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

S = math.sin(math.radians(35))
C = math.cos(math.radians(35))
RAY = np.array([0., -C, S])
SCREEN_Y = np.array([0., -S, -C])


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def project(v):
    return np.array([v[0], -v[1] * S - v[2] * C])


def build(track, depth):
    rings = []
    source_indices = []
    for section in track['sections']:
        a, b = np.array(section['endpoints_source_xy'], dtype=float)
        u2 = (b - a) / np.linalg.norm(b - a)
        u = np.array([u2[0], 0., 0.]) + SCREEN_Y * u2[1]
        rings.append({'center': np.array(section['center_world_hypothesis']), 'u': u,
                      'v': RAY, 'radius': section['projected_width_pixels'] / 2,
                      'source': section, 'role': 'proposed_source_section'})
    if track['track'] == 'main_lower':
        # Flat contact is only a scaffold. Local terrain/ivy must establish final roots.
        root = {'center': np.array([439., -depth / S, 0.]), 'u': np.array([1., 0., 0.]),
                'v': np.array([0., 1., 0.]), 'radius': 24., 'role': 'inferred_ground_scaffold'}
        rings.insert(0, root)
    else:
        vector = rings[0]['center'] - rings[1]['center']
        vector /= np.linalg.norm(vector)
        rings.insert(0, dict(rings[0], center=rings[0]['center'] + vector * rings[0]['radius'] * 2,
                             source=None, role='inferred_hidden_junction'))
    vector = rings[-1]['center'] - rings[-2]['center']
    vector /= np.linalg.norm(vector)
    rings.append(dict(rings[-1], center=rings[-1]['center'] + vector * rings[-1]['radius'] * 2,
                      source=None, role='inferred_hidden_continuation'))
    vertices, faces, endpoints = [], [], []
    n = 12
    for index, ring in enumerate(rings):
        for i in range(n):
            theta = 2 * math.pi * i / n
            vertices.append((ring['center'] + ring['radius'] *
                             (ring['u'] * math.cos(theta) + ring['v'] * math.sin(theta))).tolist())
        if ring.get('source') is not None:
            source_indices.append(index)
            wanted = ring['source']['endpoints_source_xy']
            actual = [project(vertices[index * n + n // 2]), project(vertices[index * n])]
            assert np.max(np.abs(np.array(actual) - wanted)) < 1e-9
            endpoints.extend([{'vertex': index * n + n // 2, 'source_xy': wanted[0]},
                              {'vertex': index * n, 'source_xy': wanted[1]}])
    for j in range(len(rings) - 1):
        for i in range(n):
            a, b = j * n + i, j * n + (i + 1) % n
            faces.extend([[a, b, b + n], [a, b + n, a + n]])
    for j, reverse in [(0, True), (len(rings) - 1, False)]:
        vertex = len(vertices)
        vertices.append(rings[j]['center'].tolist())
        for i in range(n):
            face = [vertex, j * n + i, j * n + (i + 1) % n]
            faces.append(face[::-1] if reverse else face)
    vv = np.array(vertices)
    volume = sum(np.dot(vv[a], np.cross(vv[b], vv[c])) / 6 for a, b, c in faces)
    if volume < 0:
        faces = [f[::-1] for f in faces]
        volume = -volume
    edge_counts = Counter(tuple(sorted((a, b))) for f in faces for a, b in zip(f, f[1:] + f[:1]))
    assert set(edge_counts.values()) == {2}
    assert len(vertices) - len(edge_counts) + len(faces) == 2
    areas = [np.linalg.norm(np.cross(vv[b] - vv[a], vv[c] - vv[a])) / 2 for a, b, c in faces]
    assert min(areas) > 1e-8 and volume > 0
    return {'track': track['track'], 'source_obstacle_context': track['native_obstacle_context'],
            'vertices': vertices, 'faces': faces, 'volume': float(volume),
            'ring_sides': n, 'source_ring_indices': source_indices,
            'ring_roles': [r['role'] for r in rings], 'source_endpoint_guards': endpoints,
            'source_endpoint_max_error': 0., 'topology_closed': True, 'euler': 2,
            'minimum_triangle_area': float(min(areas)),
            'bounds_world': [vv.min(0).tolist(), vv.max(0).tolist()],
            'section_depth_width_ratio': 1., 'source_ownership_assigned': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = json.loads(args.source.read_text())
    for path, digest in data['pins'].items():
        assert sha(Path(path)) == digest, path
    depth = data['geometry_recipe']['depth_reference_native_y']
    parts = [build(track, depth) for track in data['wood_tracks']]
    assert sum(len(p['vertices']) for p in parts) < 2500
    assert sum(len(p['faces']) for p in parts) < 5000
    report = {'status': 'HOLD_PROVISIONAL_CLOSED_PARTS_JOINTS_AND_CONTACT_UNVALIDATED',
              'source': str(args.source.resolve()), 'source_sha256': sha(args.source), 'parts': parts,
              'projection': data['geometry_recipe']['projection'],
              'assumptions': ['Each source section has a circular cross-section in the plane spanned by its projected transverse axis and native camera ray.',
                              'Thus both traced 2D section endpoints are exact and depth equals width; depth placement remains inferred.',
                              'Root ring lies on z=0 at native obstacle mean depth. This is not measured local terrain contact.',
                              'Source endpoints are approximate manual hypotheses, not texture boundaries.',
                              'Individual sweeps are closed; their assembly has not been unioned or tested for self-intersection.'],
              'joining_plan': ['Inspect overlaps at main_lower/main_upper, main trunk/left_fork and main trunk/right_fork in saved native/oblique views.',
                               'Union only verified intersecting junctions with exact Boolean; verify one connected closed component and positive volume.',
                               'Before and after union, independently compare source silhouettes and endpoint rays; never delete a source-supported fork merely to obtain a manifold result.',
                               'Keep native obstacle33/34 identities in context metadata; added branches are inferred visual components, not new gameplay obstacles.'],
              'contact_plan': ['Replace ground scaffold only after bank52/53/54, plateau94-97 and local terrain contact inspection.',
                               'Trace ivy and possible left root separately; do not project their RGB onto the whole trunk.',
                               'Retain nativeleaf single owner and foreground priority for every phase; no bark assignments made.'],
              'saved_builder': 'level-editor/blender/croisement03/restart2_tree18_save_wood.py',
              'heavy_execution': 'NOT_RUN'}
    args.output.mkdir(parents=True, exist_ok=False)
    source_path = next(Path(p) for p in data['pins'] if p.endswith('/baseline/covered.png'))
    box = (270, 260, 550, 560)
    image = Image.open(source_path).convert('RGB').crop(box).resize((840, 900), Image.Resampling.NEAREST)
    painter = ImageDraw.Draw(image)
    for part, color in zip(parts, ['cyan', 'magenta', '#5599ff', 'white']):
        points = (np.array([project(v) for v in part['vertices']]) - box[:2]) * 3
        for face in part['faces']:
            painter.line([tuple(points[i]) for i in face + [face[0]]], fill=color, width=1)
    image.save(args.output / 'native-wire-provisional.png')
    report['native_wire_sha256'] = sha(args.output / 'native-wire-provisional.png')
    (args.output / 'geometry.json').write_text(json.dumps(report, indent=2) + '\n')
    print({'parts': len(parts), 'vertices': sum(len(p['vertices']) for p in parts),
           'triangles': sum(len(p['faces']) for p in parts), 'status': report['status']})


if __name__ == '__main__':
    main()
