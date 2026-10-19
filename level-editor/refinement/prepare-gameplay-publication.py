"""Prepare a publication subset from an immutable browser gameplay snapshot."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile

from asset_index import validate_asset_index
from promote_staged_publication import safe_relative

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from lossy_assets import verify_derivatives


def digest(data):
    return hashlib.sha256(data).hexdigest()


def prepare(fixture, gameplay_stage):
    fixture, gameplay_stage = Path(fixture).resolve(), Path(gameplay_stage).resolve()
    edits = json.loads((gameplay_stage / 'edits.json').read_bytes())
    by_id = {edit['asset']: edit for edit in edits}
    assert len(by_id) == len(edits)
    source_root = fixture / '3d-assets'
    catalog = json.loads((source_root / 'index.json').read_bytes())
    catalog['assets'] = [entry for entry in catalog['assets'] if entry['id'] in by_id]
    assert len(catalog['assets']) == len(edits)
    output = Path(tempfile.mkdtemp(prefix='reviewed-publication-stage-', dir=gameplay_stage.parent))
    target_root = output / '3d-assets'
    target_root.mkdir()
    directories = set()
    for entry in catalog['assets']:
        edit = by_id[entry['id']]
        descriptor_path = safe_relative(entry['descriptor'])
        directory = descriptor_path.parent
        assert directory not in directories, 'Selected descriptors must have separate directories'
        directories.add(directory)
        descriptor_bytes = (source_root / descriptor_path).read_bytes()
        descriptor = json.loads(descriptor_bytes)
        assert descriptor['gameplay'] == edit['gameplay']
        assert digest(descriptor_bytes) == entry['descriptor_sha256']
        assert digest((source_root / safe_relative(entry['model'])).read_bytes()) == edit['modelSha256']
        receipt_path = directory / 'bundle.receipt.json'
        rebind_receipt = descriptor.get('model_scene') is not None and 'resources' not in descriptor
        for source in (source_root / directory).rglob('*'):
            if not source.is_file():
                continue
            relative = source.relative_to(source_root)
            if rebind_receipt and relative == receipt_path:
                continue
            target = target_root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            os.link(source, target)
        if rebind_receipt:
            receipt = json.loads((source_root / receipt_path).read_bytes())
            assert receipt['output']['sha256'] == edit['modelSha256']
            assert receipt['output_descriptor_sha256'] == edit['descriptorSha256']
            receipt['output_descriptor_sha256'] = digest(descriptor_bytes)
            (target_root / receipt_path).write_text(json.dumps(receipt, indent=2) + '\n')
        for resource in descriptor.get('resources', []):
            relative = safe_relative(resource['path'])
            source = fixture / relative
            assert digest(source.read_bytes()) == resource['sha256']
            target = output / 'map-assets' / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            if not target.exists():
                os.link(source, target)
    (target_root / 'index.json').write_text(json.dumps(catalog))
    validate_asset_index(target_root, catalog)
    problems = verify_derivatives(target_root, index=catalog)
    assert not problems, problems
    (output / 'preparation.json').write_text(json.dumps({
        'fixture': str(fixture), 'gameplayStage': str(gameplay_stage),
        'assets': sorted(by_id), 'scope': 'publication-inputs-not-browser-or-gameplay-proof',
    }, indent=2))
    return output


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('fixture')
    parser.add_argument('gameplay_stage')
    args = parser.parse_args()
    print(prepare(args.fixture, args.gameplay_stage))
