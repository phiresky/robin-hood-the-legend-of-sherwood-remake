"""Stage reusable spline collision/calibration from asset models, never level records."""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import numpy as np


def clip_polygon(vertices, axis, boundary, above):
    result = []
    for a, b in zip(vertices, vertices[1:] + vertices[:1]):
        ia = a[axis] >= boundary if above else a[axis] <= boundary
        ib = b[axis] >= boundary if above else b[axis] <= boundary
        if ia:
            result.append(a)
        if ia != ib:
            result.append(a + (b - a) * ((boundary - a[axis]) / (b[axis] - a[axis])))
    return result


def strip_gameplay(triangles, node, *, material, opaque, elevation=35):
    """Conservative, ground-sealed barrier bands; no inferred wall-top navigation."""
    points = np.concatenate(triangles)
    lo, hi = points.min(axis=0), points.max(axis=0)
    if np.any(hi - lo <= 1e-5):
        raise ValueError('Wall source needs nonzero length, thickness and height')
    cuts = np.linspace(lo[0], hi[0], min(128, max(1, math.ceil((hi[0] - lo[0]) / 4))) + 1)
    bands = []
    for a, b in zip(cuts, cuts[1:]):
        clipped = []
        for triangle in triangles:
            if triangle[:, 0].max() < a or triangle[:, 0].min() > b:
                continue
            poly = clip_polygon(clip_polygon(list(triangle), 0, a, True), 0, b, False)
            if len(poly) >= 3:
                clipped.extend(poly)
        if not clipped:
            continue
        vertices = np.array(clipped)
        bottom, top = vertices.min(axis=0), vertices.max(axis=0)
        if top[1] - bottom[1] <= 1e-5 or top[2] - lo[2] <= 1e-5:
            continue
        band = [float(a), float(b), float(bottom[1]), float(top[1]), float(top[2])]
        if bands and abs(bands[-1][1] - a) < 1e-6 and np.allclose(bands[-1][2:], band[2:], atol=1e-5, rtol=0):
            bands[-1][1] = float(b)
        else:
            bands.append(band)
    if not bands:
        raise ValueError('Wall source has no physical barrier bands')
    sine, cosine = math.sin(math.radians(elevation)), math.cos(math.radians(elevation))
    volumes = []
    for i, (a, b, left, right, top) in enumerate(bands):
        volumes.append({'id': f'barrier-{i}', 'node': node, 'shape': {
            'points': [{'x': x, 'y': -y*sine, 'z_bottom': float(lo[2])*cosine, 'z_top': top*cosine}
                       for x, y in [(a, left), (b, left), (b, right), (a, right)]],
            'solid': True, 'opaque': opaque, 'mouse': True,
            'show_shadow_polygon': False, 'default_material': material}})
    return {'version': 1, 'collision': 'none', 'surfaces': [], 'doors': [], 'volumes': volumes,
            'spline': {'bounds': {'min': lo.tolist(), 'max': hi.tolist()},
                       'frames': {node: np.eye(4).flatten(order='F').tolist()}},
            'draft': {'issues': ['Spline collision uses a conservative continuous barrier envelope; openings and walkable tops require authored gameplay surfaces/volumes.']}}


def author(library, presets, output, corners=()):
    from build_segments import primitives, read_glb, node_matrix
    root = library / '3d-assets'
    index = json.loads((root / 'index.json').read_text())['assets']
    entries = {entry['id']: entry for entry in index}
    edits = []
    strip_ids = {row['asset'] for row in presets}
    policies = {row['id']: row['collision'] for row in json.loads((Path(__file__).parent / 'recipes.json').read_text())}
    corner_ids = {row['cornerAsset'] for row in presets if row.get('cornerAsset')} | set(corners)
    for identity in sorted(strip_ids | corner_ids):
        entry = entries[identity]
        raw = (root / entry['descriptor']).read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        if digest != entry['descriptor_sha256']:
            raise ValueError('Stale descriptor: ' + identity)
        descriptor = json.loads(raw)
        doc, buffers, _ = read_glb(root / entry['model'])
        parts = primitives(doc, buffers, descriptor.get('model_scene'),
                           [p['node'] for p in descriptor['parts'] if p.get('default_hidden')])
        triangles = [attrs['POSITION'][indices] for attrs, ids, _, _ in parts for indices in ids]
        if identity in strip_ids:
            gameplay = strip_gameplay(triangles, descriptor['parts'][0]['node'], **policies[identity])
        else:
            gameplay = copy.deepcopy(descriptor['gameplay'])
            frames = {}
            expected = {part['node'] for part in descriptor['parts']}
            scene = next(s for s in doc['scenes'] if s.get('name') == descriptor['model_scene']) if descriptor.get('model_scene') else doc['scenes'][doc.get('scene', 0)]
            def walk(i, parent):
                node = doc['nodes'][i]
                matrix = parent if node.get('name') == 'map' and 'mesh' not in node else parent @ node_matrix(node)
                if node.get('name') in expected:
                    if node['name'] in frames:
                        raise ValueError('Duplicate model frame: ' + node['name'])
                    frames[node['name']] = matrix.flatten(order='F').tolist()
                for child in node.get('children', []):
                    walk(child, matrix)
            for i in scene['nodes']:
                walk(i, np.eye(4))
            if expected != set(frames):
                raise ValueError('Missing asset frames: ' + identity)
            points = np.concatenate(triangles)
            gameplay['spline'] = {'bounds': {'min': points.min(axis=0).tolist(), 'max': points.max(axis=0).tolist()}, 'frames': frames}
        gameplay['spline']['modelSha256'] = hashlib.sha256((root / entry['model']).read_bytes()).hexdigest()
        edits.append({'asset': identity, 'descriptorSha256': digest, 'gameplay': gameplay})
        print(identity, len(gameplay.get('volumes', [])), 'volumes')
    with output.open('x') as target:
        json.dump(edits, target, indent=2)
        target.write('\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--library', type=Path, default=Path('library'))
    parser.add_argument('--presets', type=Path, default=Path('app/src/assets/wall-presets.json'))
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--corners', nargs='*', default=[], help='Additional selectable corner assets to calibrate')
    args = parser.parse_args()
    author(args.library, json.loads(args.presets.read_text()), args.output, args.corners)
