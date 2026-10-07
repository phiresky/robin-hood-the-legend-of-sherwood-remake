"""Publish the reviewed bark delta with its exact scoped catalog and rollback."""
import argparse
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
import promote_staged_publication as publisher


def read(path):
    return json.loads(path.read_text())


def validate(stage):
    review_path = stage / 'root-publication-review.json'
    review = read(review_path)
    if not review['status'].startswith('PASS'):
        raise ValueError('Independent root review required')
    for filename, digest in review['evidence'].items():
        if publisher.sha(Path(filename)) != digest:
            raise ValueError('Reviewed evidence changed: ' + filename)
    verification = read(stage / 'editor-review-v2/verification.json')
    pins = read(stage / 'editor-review-v2/prelaunch-pins.json')
    if not verification['status'].startswith('PASS'):
        raise ValueError('Staged Editor verification required')
    for filename, digest in pins.items():
        if publisher.sha(Path(filename)) != digest:
            raise ValueError('Staged browser input changed: ' + filename)
    candidate = read(stage / 'candidate.json')
    if publisher.sha(stage / 'candidate.json') != review['candidate_sha256']:
        raise ValueError('Candidate review mismatch')
    if candidate['inherited_derivative_holds'] or not all(candidate[key] for key in (
        'placements_exact', 'gameplay_descriptors_exact', 'other_map_index_entries_exact',
        'other_asset_model_pins_exact', 'only_five_source_model_pins_changed')):
        raise ValueError('Candidate scope or derivative check failed')
    manifest = read(stage / 'promotion-draft.json')
    if manifest['status'] != 'PENDING_BROWSER_NOT_APPLIED':
        raise ValueError('Expected frozen unapplied draft')
    library = Path(manifest['library'])
    index_target = str(library / '3d-assets/index.json')
    index_row = next(row for row in manifest['files'] if row['target'] == index_target)
    # Preserve the entire reviewed catalog. Generic regeneration can refresh
    # unrelated entries whose embedded metadata is intentionally unchanged here.
    manifest.pop('index_generation')
    manifest['files'] = [row for row in manifest['files'] if row is not index_row] + [index_row]
    manifest['status'] = 'PREPARED_NOT_APPLIED'
    manifest['browser_check'] = {'status': 'PASS', 'verification_sha256': publisher.sha(stage / 'editor-review-v2/verification.json')}
    manifest['root_review_sha256'] = publisher.sha(review_path)
    manifest['exact_catalog'] = True
    for row in manifest['files']:
        if row['source'] is None:
            raise ValueError('This delta authorizes no deletions')
        if publisher.sha(Path(row['source'])) != row['source_sha256'] or publisher.sha(Path(row['target'])) != row['previous_sha256']:
            raise ValueError('Publication bytes changed: ' + row['target'])
        publisher.check_gameplay_preserved(Path(row['source']), Path(row['target']))
    return manifest, index_row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', type=Path)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    stage = args.stage.resolve()
    manifest, index_row = validate(stage)
    if not args.apply:
        print('PASS reviewed delta preflight:', len(manifest['files']), 'files')
        return
    output = stage / 'promotion.json'
    if output.exists():
        raise FileExistsError(output)
    with publisher.library_lock(Path(manifest['library'])):
        manifest, index_row = validate(stage)
        output.write_text(json.dumps(manifest, indent=2) + '\n')
        original = publisher.write_asset_index
        def install_exact_index(asset_root, **kwargs):
            target = Path(index_row['target'])
            source = Path(index_row['source'])
            if kwargs or Path(asset_root).resolve() != target.parent.resolve():
                raise ValueError('Unexpected catalog regeneration request')
            if publisher.sha(target) != index_row['previous_sha256'] or publisher.sha(source) != index_row['source_sha256']:
                raise ValueError('Catalog changed before install')
            temporary = target.with_name(target.name + '.publication-tmp')
            if temporary.exists():
                raise FileExistsError(temporary)
            shutil.copy2(source, temporary)
            temporary.replace(target)
        publisher.write_asset_index = install_exact_index
        try:
            publisher._apply(output)
        finally:
            publisher.write_asset_index = original
    for row in read(output)['files']:
        if publisher.sha(Path(row['target'])) != row['source_sha256']:
            raise ValueError('Installed byte verification failed')
    print('PASS all installed transaction bytes verified')


if __name__ == '__main__':
    main()
