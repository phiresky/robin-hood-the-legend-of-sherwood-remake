"""Prove complete native occlusion of projected foliage faces; never edits workers."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import shapely
from shapely.geometry import box
from native_occlusion_proof import ObservedOcclusion, opaque_footprint


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main(source, destination, all_fronts=False):
    if destination.exists():
        raise ValueError('Use a fresh proof destination')
    input_path = source / 'input.json'
    payload = json.loads(input_path.read_text())
    polygons = [triangle['polygon'] for triangle in payload['triangles']]
    if len(set(polygons)) != len(polygons):
        raise ValueError('Whole-face proof requires triangular polygons, not triangulated quads')
    alpha_path = source / 'physical-alpha.npz'
    if sha(alpha_path) != payload['alpha_sha256']:
        raise ValueError('Physical alpha export changed')
    worker = Path(payload['worker'])
    if sha(worker / 'model.blend') != payload['model_sha256']:
        raise ValueError('Input worker changed')
    binary_audit = source / 'alpha-binary-audit.json'
    if not all(image.get('binary_alpha') for image in payload['images'].values()):
        check = json.loads(binary_audit.read_text())
        if check['input_sha256'] != sha(input_path) or check['model_sha256'] != payload['model_sha256']:
            raise ValueError('Binary alpha audit is stale')
        by_name = {image['image']: image for image in check['images']}
        for name, image in payload['images'].items():
            item = by_name[name]
            if (not item['binary_alpha'] or not set(item['alpha_values']) <= {0, 255}
                    or item['packed_sha256'] != image['packed_sha256']
                    or sha(Path(item['matching_file'])) != image['packed_sha256']):
                raise ValueError('Physical occluder alpha is not proven binary')
    with np.load(alpha_path) as archive:
        alphas = {key: archive[key] for key in archive.files}
    ray = np.asarray(payload['ray'])
    viewport = box(*payload['native_viewport'])
    occluders, depths, occluder_ids, targets = [], [], [], []
    for index, triangle in enumerate(payload['triangles']):
        if index % 1000 == 0:
            print(f'Physical opacity footprint {index}/{len(payload["triangles"])}', flush=True)
        points = np.asarray(triangle['points'])
        normal = np.cross(points[1] - points[0], points[2] - points[0])
        if normal.dot(ray) <= 0:
            raise ValueError('Expected native-front-facing physical triangles')
        projected = np.c_[points[:, 0], -points[:, 1] * payload['sin'] - points[:, 2] * payload['cos']]
        image = payload['images'][triangle['image']]
        footprint = opaque_footprint(triangle['uv'], alphas[image['alpha_key']], projected, image['extension'])
        footprint = footprint.intersection(viewport)
        depth = points @ ray
        if triangle['slot'] == 0 or all_fronts:
            occluders.append(footprint)
            depths.append(float(depth.min()))
            occluder_ids.append(triangle['polygon'])
        if triangle['slot'] == 5:
            targets.append((triangle, footprint, float(depth.max())))
    authority = ObservedOcclusion(occluders, depths)
    results = []
    for index, (triangle, footprint, depth) in enumerate(targets):
        result = authority.prove(footprint, depth)
        result['occluders'] = [occluder_ids[i] for i in result['occluders']]
        result.update(polygon=triangle['polygon'], image=triangle['image'], depth_maximum=depth)
        results.append(result)
        if index % 500 == 0:
            print(f'Continuous occlusion proof {index}/{len(targets)}', flush=True)
    if sha(worker / 'model.blend') != payload['model_sha256']:
        raise ValueError('Proof changed worker')
    destination.mkdir(parents=True)
    result = dict(input=str(input_path), input_sha256=sha(input_path), alpha_sha256=sha(alpha_path),
        worker=str(worker), model_sha256=payload['model_sha256'], worker_unchanged=True,
        method='Exact nearest-alpha texel footprints in the native viewport; each target full opaque footprint must be covered by strictly closer physical cutouts.',
        occluder_slots=[0, 5] if all_fronts else [0],
        simultaneous_rgb_change_rule='Geometry and binary alpha remain immutable. Strict minimum-occluder versus maximum-target depth ordering forbids cycles and guarantees a retained first-hit surface.',
        binary_alpha_audit_sha256=sha(binary_audit) if binary_audit.exists() else None,
        depth_margin=.05, projection_margin=.002, shapely_version=shapely.__version__,
        target_faces=len(targets), completely_occluded_faces=sum(r['hidden'] for r in results),
        hidden_polygons=[r['polygon'] for r in results if r['hidden']], results=results,
        approval='Read-only geometric proof. No ownership or texture edited.',
        limitation='Only the declared native orthographic direction and viewport are proven. New derivatives still require exact alpha/geometry/observed texture preservation and independent actual-view review.')
    (destination / 'evidence.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: result[k] for k in ('target_faces', 'completely_occluded_faces', 'model_sha256')}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--all-fronts', action='store_true', help='Permit strictly closer original projected fronts as immutable-alpha occluders')
    args = parser.parse_args()
    main(args.source.resolve(), args.destination.resolve(), args.all_fronts)
