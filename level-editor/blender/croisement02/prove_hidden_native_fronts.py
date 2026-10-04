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


def main(source, destination, all_fronts=False, resume=False):
    if destination.exists() and (not resume or (destination / 'evidence.json').exists()):
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
    identity = dict(input_sha256=sha(input_path), model_sha256=payload['model_sha256'],
        all_fronts=all_fronts, recipe_sha256=sha(Path(__file__)),
        algorithm_sha256=sha(Path(__file__).with_name('native_occlusion_proof.py')))
    destination.mkdir(parents=True, exist_ok=resume)
    identity_path = destination / 'checkpoint-identity.json'
    if resume:
        if json.loads(identity_path.read_text()) != identity:
            raise ValueError('Checkpoint inputs or proof algorithm changed')
    else:
        identity_path.write_text(json.dumps(identity, indent=2) + '\n')
    def progress(stage, completed, total):
        path = destination / 'progress.json'
        temporary = path.with_suffix('.tmp')
        temporary.write_text(json.dumps(dict(status='INCOMPLETE', stage=stage,
            completed=completed, total=total, **identity), indent=2) + '\n')
        temporary.replace(path)
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
    cache = destination / 'physical-footprints.npz'
    footprints = []
    if resume and cache.exists():
        with np.load(cache, allow_pickle=False) as stored:
            blob, offsets = stored['blob'].tobytes(), stored['offsets']
            footprints = [shapely.from_wkb(blob[a:b]) for a, b in zip(offsets[:-1], offsets[1:])]
        if len(footprints) != len(payload['triangles']):
            raise ValueError('Incomplete physical footprint cache')
    for index, triangle in enumerate(payload['triangles']):
        if index % 1000 == 0:
            print(f'Physical opacity footprint {index}/{len(payload["triangles"])}', flush=True)
            progress('physical footprints', index, len(payload['triangles']))
        points = np.asarray(triangle['points'])
        normal = np.cross(points[1] - points[0], points[2] - points[0])
        if normal.dot(ray) <= 0:
            raise ValueError('Expected native-front-facing physical triangles')
        projected = np.c_[points[:, 0], -points[:, 1] * payload['sin'] - points[:, 2] * payload['cos']]
        image = payload['images'][triangle['image']]
        if index < len(footprints):
            footprint = footprints[index]
        else:
            footprint = opaque_footprint(triangle['uv'], alphas[image['alpha_key']], projected, image['extension'])
            footprint = footprint.intersection(viewport)
            footprints.append(footprint)
        depth = points @ ray
        if triangle['slot'] == 0 or all_fronts:
            occluders.append(footprint)
            depths.append(float(depth.min()))
            occluder_ids.append(triangle['polygon'])
        if triangle['slot'] == 5:
            targets.append((triangle, footprint, float(depth.max())))
    if not cache.exists():
        encoded = [shapely.to_wkb(p) for p in footprints]
        offsets = np.cumsum([0] + [len(p) for p in encoded], dtype=np.int64)
        np.savez_compressed(cache, blob=np.frombuffer(b''.join(encoded), dtype=np.uint8), offsets=offsets)
    authority = ObservedOcclusion(occluders, depths)
    checkpoint = destination / 'partial-results.jsonl'
    results = [json.loads(line) for line in checkpoint.read_text().splitlines()] if resume and checkpoint.exists() else []
    if any(r['polygon'] != target[0]['polygon'] for r, target in zip(results, targets)) or len(results) > len(targets):
        raise ValueError('Checkpoint face order differs from proof inputs')
    for index, (triangle, footprint, depth) in enumerate(targets):
        if index < len(results):
            continue
        result = authority.prove(footprint, depth)
        result['occluders'] = [occluder_ids[i] for i in result['occluders']]
        result.update(polygon=triangle['polygon'], image=triangle['image'], depth_maximum=depth)
        results.append(result)
        with checkpoint.open('a') as stream:
            stream.write(json.dumps(result) + '\n')
        if index % 50 == 0:
            print(f'Continuous occlusion proof {index}/{len(targets)}', flush=True)
            progress('continuous coverage', index + 1, len(targets))
    if sha(worker / 'model.blend') != payload['model_sha256']:
        raise ValueError('Proof changed worker')
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
    (destination / 'progress.json').write_text(json.dumps(dict(status='COMPLETE',
        evidence_sha256=sha(destination / 'evidence.json'), **identity), indent=2) + '\n')
    print(json.dumps({k: result[k] for k in ('target_faces', 'completely_occluded_faces', 'model_sha256')}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--all-fronts', action='store_true', help='Permit strictly closer original projected fronts as immutable-alpha occluders')
    parser.add_argument('--resume', action='store_true', help='Resume only a hash-identical incomplete checkpoint; partial results are never final evidence')
    args = parser.parse_args()
    main(args.source.resolve(), args.destination.resolve(), args.all_fronts, args.resume)
