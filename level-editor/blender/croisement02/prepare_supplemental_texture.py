"""Prepare source-only derivatives of approved geometry and camera evidence.

The original user geometry decision remains provenance. Newly rendered source
pixels are explicitly a reviewed preparation derivative, not a new user decision.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from prepare_tree_texture import attach_references, checked, prepare, read, require, sha, write


def main(original, packet, review_path, output):
    original, packet = original.resolve(strict=True), packet.resolve(strict=True)
    if (original / 'preparation.json').exists():
        receipt = read(original / 'preparation.json')
        manifest_path = checked(receipt['source_review_manifest'], receipt['review_manifest_sha256'])
        source_records = [('source-preparation', original / 'preparation.json'), ('source-approval', original / 'approval.json')]
    else:
        # A technically valid small original packet may have a schema bridge
        # but no generation experiment until a new render meets transport size.
        manifest_path = (original / 'review-manifest.json').resolve(strict=True)
        source_records = [('source-manifest', manifest_path), ('source-decisions', original / 'decisions.json')]

    data = read(manifest_path)
    item = copy.deepcopy(data['items'][0])
    asset = item['id']
    derivative_frames = read(packet / 'views.json')
    width, height = derivative_frames['tile_size']
    prepare(manifest_path, asset, output / 'check-only', manifest_path.parent / 'decisions.json',
            check_only=True, check_only_atlas_size=(width * 4, height * 2))
    derivation = read(packet / 'derivation.json')
    require(derivation['asset_id'] == asset and derivation['status'] == 'PASS', 'Invalid source derivative')
    require(derivation['model_sha256'] == item['revision']['model_sha256'], 'Derivative geometry differs')
    require(derivation['geometry_and_appearance_unchanged'], 'Geometry preservation missing')
    checked(derivation['approved_camera_manifest'], derivation['approved_camera_manifest_sha256'])
    if 'approved_supplemental_audit' in derivation:
        checked(derivation['approved_supplemental_audit'], derivation['approved_supplemental_audit_sha256'])
    else:
        checked(Path(derivation['approved_original_archive']) / 'decision.json', derivation['approved_original_decision_sha256'])
    for relative, expected in derivation['artifacts'].items():
        checked(packet / relative, expected)
    review = read(review_path)
    require(review['asset_id'] == asset and review.get('ready_for_texture_preparation') is True and bool(review.get('reviewer')), 'Derived input review incomplete')
    checked(packet / 'derivation.json', review['derivation_sha256'])
    frames = read(packet / 'views.json')
    previous = read(Path(item['textured']).parent / 'views.json')
    for field in ('asset_id', 'scene_name', 'collection_name', 'source_mask_manifest', 'source_mask_evidence', 'projection_layers'):
        require(frames[field] == previous[field], 'Derivative source or ownership contract changed: ' + field)
    require(set(frames['object_names']) == set(previous['object_names']), 'Derivative receiver scope changed')
    require(not output.exists(), 'Use a new output directory')
    output.mkdir(parents=True)
    evidence = item['revision']['evidence']
    for key, path in source_records + [('derived-input-review', review_path)]:
        evidence[key] = {'path': str(path.resolve()), 'sha256': sha(path)}
    for path in sorted(packet.rglob('*')):
        if path.is_file():
            evidence['supplemental/' + str(path.relative_to(packet))] = {'path': str(path), 'sha256': sha(path)}
    item.update(solid=str(packet / 'solid.png'), textured=str(packet / 'textured.png'), context=str(packet / 'context.png'))
    item['approval_provenance'] = {'original_geometry_authorization': item['approval_provenance'],
        'derivation': derivation, 'preparation_review': review,
        'scope': 'Original user-approved model; derived framing and render resolution are explicitly recorded and reviewed for texture preparation. Newly rendered source pixels are not represented as separately user-approved pixels.'}
    identity = {'asset_id': asset, 'model_sha256': item['revision']['model_sha256'],
                'evidence': {key: value['sha256'] for key, value in evidence.items()}}
    item['revision']['sha256'] = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    decision = copy.deepcopy(read(manifest_path.parent / 'decisions.json')['decisions'][-1])
    decision['revision_sha256'] = item['revision']['sha256']
    decision['translation'] = 'Original approved geometry with explicitly recorded, separately reviewed source-only framing/resolution derivative; no additional user approval inferred.'
    write(output / 'review-manifest.json', dict(version=1, items=[item]))
    write(output / 'decisions.json', dict(version=1, decisions=[decision]))
    result = prepare(output / 'review-manifest.json', asset, output / 'experiment', output / 'decisions.json')
    attach_references(output / 'experiment')
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original_experiment', type=Path)
    parser.add_argument('derived_packet', type=Path)
    parser.add_argument('input_review', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    main(args.original_experiment, args.derived_packet, args.input_review, args.output.resolve())
