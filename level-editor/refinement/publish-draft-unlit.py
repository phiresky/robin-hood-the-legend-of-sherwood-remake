"""Install a verified draft material repair and its freshly generated derivatives."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from lossy_assets import LibraryLock, atomic_write, verify_derivatives
from asset_index import validate_asset_index


def digest(data):
    return hashlib.sha256(data).hexdigest()


def unpack(data):
    magic, version, length, size, kind = struct.unpack_from('<5I', data)
    assert (magic, version, length, kind) == (0x46546c67, 2, len(data), 0x4e4f534a)
    return json.loads(data[20:20 + size]), data[20 + size:]


def publish(library, stage, output, apply=False):
    library, stage, output = map(lambda p: Path(p).resolve(), (library, stage, output))
    assert not output.is_relative_to(library)
    output.mkdir(parents=True, exist_ok=False)
    review = json.loads((stage / 'report.json').read_text())
    staged_root = stage / '3d-assets'
    assert not verify_derivatives(staged_root)
    entries = json.loads((staged_root / 'index.json').read_text())['assets']
    assert len(entries) == 1 and entries[0]['id'] == review['asset']
    entry = entries[0]
    root = library / '3d-assets'
    model_path = Path(entry['model'])
    assert not model_path.is_absolute() and '..' not in model_path.parts
    with LibraryLock(root):
        old_model = (root / model_path).read_bytes()
        new_model = (staged_root / model_path).read_bytes()
        assert digest(old_model) == review['sourceSha256']
        assert digest(new_model) == review['modelSha256']
        descriptor = (root / entry['descriptor']).read_bytes()
        assert digest(descriptor) == review['descriptorSha256']
        assert descriptor == (staged_root / entry['descriptor']).read_bytes()
        old, old_binary = unpack(old_model)
        new, new_binary = unpack(new_model)
        expected = copy.deepcopy(old)
        for index in review['changedMaterials']:
            material = expected['materials'][index]
            assert material['name'] == 'Unfinished volume preview'
            assert 'KHR_materials_unlit' not in material.get('extensions', {})
            material.setdefault('extensions', {})['KHR_materials_unlit'] = {}
        expected['extensionsUsed'] = list(dict.fromkeys(expected.get('extensionsUsed', []) + ['KHR_materials_unlit']))
        assert new == expected and new_binary == old_binary
        names = [str(model_path), entry['lossy_model'], entry['preview_model']]
        names += [name + '.receipt.json' for name in names[1:]]
        assert all(not Path(name).is_absolute() and '..' not in Path(name).parts for name in names)
        staged = {name: staged_root / name for name in names}
        replacements = {str(Path('3d-assets') / name): path.read_bytes() for name, path in staged.items()}
        before = {name: (library / name).read_bytes() for name in replacements}
        before[str(Path('3d-assets') / model_path)] = old_model
        scenes = []
        for path in sorted((library / 'scenes').glob('*.rhlos-map.json')):
            scene_before = path.read_bytes()
            document = json.loads(scene_before)
            changed = False
            for reference in document.get('assetSources', []) + document.get('sceneAssets', []):
                if reference.get('id') != review['asset']:
                    continue
                assert reference['model'] == str(Path('3d-assets') / model_path)
                assert reference['model_sha256'] == review['sourceSha256']
                assert reference['descriptor_sha256'] == review['descriptorSha256']
                reference['model_sha256'] = review['modelSha256']
                changed = True
            if changed:
                name = str(path.relative_to(library))
                before[name] = scene_before
                updated = scene_before.replace(review['sourceSha256'].encode(), review['modelSha256'].encode())
                assert json.loads(updated) == document, 'Unexpected source hash outside asset pins'
                replacements[name] = updated
                scenes.append(name)
        assert scenes, 'No saved scene pins this draft'
        index_before = (root / 'index.json').read_bytes()
        index = json.loads(index_before)
        matching = [row for row in index['assets'] if row['id'] == review['asset']]
        assert len(matching) == 1
        current = matching[0]
        assert current['model'] == entry['model'] and current['descriptor'] == entry['descriptor']
        assert current['descriptor_sha256'] == review['descriptorSha256']
        current['model_sha256'] = review['modelSha256']
        for key in ['lossy_model', 'preview_model']:
            current[key] = entry[key]
        validate_asset_index(root, {'version': 1, 'assets': [current]}, files=staged)
        replacements['3d-assets/index.json'] = (json.dumps(index, separators=(',', ':')) + '\n').encode()
        before['3d-assets/index.json'] = index_before
        for name, data in before.items():
            backup = output / 'before' / name
            backup.parent.mkdir(parents=True, exist_ok=True)
            backup.write_bytes(data)
        report = {'scope': 'single-asset-material-publication-unrelated-catalog-entries-preserved',
                  'asset': review['asset'], 'scenes': scenes, 'applied': False,
                  'modelSha256': review['modelSha256'], 'files': list(replacements)}
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        if apply:
            assert (root / entry['descriptor']).read_bytes() == descriptor, 'Concurrent descriptor edit'
            for name, data in before.items():
                assert (library / name).read_bytes() == data, f'Concurrent edit: {name}'
            installed = []
            try:
                for name, data in replacements.items():
                    atomic_write(library / name, data)
                    installed.append(name)
                assert not verify_derivatives(root, index={'assets': [entry]})
            except BaseException:
                for name in reversed(installed):
                    atomic_write(library / name, before[name])
                raise
            report['applied'] = True
            (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path, default=Path('library'))
    parser.add_argument('--stage', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    print(json.dumps(publish(args.library, args.stage, args.output, args.apply), indent=2))
