"""Install browser-verified asset repairs, derivatives and saved-scene pins together."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import zipfile

from asset_index import validate_asset_index
from promote_staged_publication import asset_file_pairs, safe_relative

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from lossy_assets import LibraryLock, atomic_write, verify_derivatives


def digest(data):
    return hashlib.sha256(data).hexdigest()


def install(library, before, replacements):
    """Preflight all targets, then restore prior bytes if an atomic replacement fails."""
    for name, previous in before.items():
        target = library / name
        assert (target.read_bytes() if target.exists() else None) == previous, f'Concurrent edit: {name}'
    written = []
    try:
        for name, data in replacements.items():
            atomic_write(library / name, data)
            written.append(name)
    except BaseException:
        for name in reversed(written):
            if before[name] is None:
                (library / name).unlink()
            else:
                atomic_write(library / name, before[name])
        raise


def publish(library, stage, gameplay_stage, fixture, output, apply=False, archive_name=None):
    library, stage, gameplay_stage, fixture, output = map(
        lambda value: Path(value).resolve(), (library, stage, gameplay_stage, fixture, output))
    assert not output.is_relative_to(library)
    output.mkdir(parents=True, exist_ok=False)
    staged_root = stage / '3d-assets'
    index = json.loads((staged_root / 'index.json').read_bytes())
    entries = index['assets']
    edits = {edit['asset']: edit for edit in json.loads((gameplay_stage / 'edits.json').read_bytes())}
    assert len(entries) == len(edits) and {entry['id'] for entry in entries} == edits.keys()
    assert not verify_derivatives(staged_root)
    validate_asset_index(staged_root, index)
    evidence = json.loads((fixture / 'browser-roundtrip-evidence.json').read_bytes())
    assert evidence['browserRoundtrip'].startswith('PASS ')
    if archive_name is not None:
        assert Path(archive_name).name == archive_name, 'Archive must be a fixture filename'
    archives = [fixture / archive_name] if archive_name is not None else list(fixture.glob('*.zip'))
    assert len(archives) == 1
    archive = archives[0].read_bytes()
    assert digest(archive) == evidence['zipSha256'] and len(archive) == evidence['bytes']
    with zipfile.ZipFile(archives[0]) as zipped:
        names = [name for name in zipped.namelist() if name.startswith('editor/') and name.endswith('.rhlos-map.json')]
        assert len(names) == 1
        document = json.loads(zipped.read(names[0]))
    assert not document.get('mission')
    references = {ref['id']: ref for ref in document['assetSources']}
    root = library / '3d-assets'
    with LibraryLock(root):
        index_before = (root / 'index.json').read_bytes()
        live = json.loads(index_before)
        replacements = {}
        pins = {}
        for entry in entries:
            asset = entry['id']
            edit = edits[asset]
            current = next(row for row in live['assets'] if row['id'] == asset)
            assert current['descriptor'] == entry['descriptor'] and current['model'] == entry['model']
            descriptor_path = safe_relative(entry['descriptor'])
            model_path = safe_relative(entry['model'])
            old = (root / descriptor_path).read_bytes()
            new = (staged_root / descriptor_path).read_bytes()
            model = (staged_root / model_path).read_bytes()
            assert digest(old) == edit['descriptorSha256'] == current['descriptor_sha256']
            assert digest((root / model_path).read_bytes()) == edit['modelSha256']
            old_descriptor, new_descriptor = json.loads(old), json.loads(new)
            assert {k: v for k, v in old_descriptor.items() if k != 'gameplay'} == {
                k: v for k, v in new_descriptor.items() if k != 'gameplay'}
            assert new_descriptor['gameplay'] == edit['gameplay']
            assert new_descriptor['gameplay'].get('draft', {}).get('issues'), 'Retain draft limitations'
            ref = references[asset]
            assert ref['descriptor_sha256'] == digest(new) and ref['model_sha256'] == digest(model)
            assert (fixture / ref['descriptor']).read_bytes() == new
            assert (fixture / ref['model']).read_bytes() == model
            for source, target in asset_file_pairs(staged_root, root, entry):
                replacements[str(target.relative_to(library))] = source.read_bytes()
            for field in ('lossy_model', 'preview_model'):
                assert entry.get(field), f'Missing {field}: {asset}'
                for suffix in ('', '.receipt.json'):
                    relative = safe_relative(entry[field] + suffix)
                    data = (staged_root / relative).read_bytes()
                    assert (fixture / '3d-assets' / relative).read_bytes() == data
                    replacements[str(Path('3d-assets') / relative)] = data
                current[field] = entry[field]
            current['editor'] = entry['editor']
            current['descriptor_sha256'] = digest(new)
            current['model_sha256'] = digest(model)
            pins[asset] = (str(Path('3d-assets') / descriptor_path), str(Path('3d-assets') / model_path),
                           edit['descriptorSha256'], digest(new), edit['modelSha256'], digest(model))
        scenes = []
        for path in sorted((library / 'scenes').glob('*.rhlos-map.json')):
            original = path.read_bytes()
            scene = json.loads(original)
            updated = original
            for ref in scene.get('assetSources', []) + scene.get('sceneAssets', []):
                if ref.get('id') not in pins:
                    continue
                descriptor, model, old_desc, new_desc, old_model, new_model = pins[ref['id']]
                assert ref['descriptor'] == descriptor and ref['model'] == model
                assert ref['descriptor_sha256'] == old_desc and ref['model_sha256'] == old_model
                ref['descriptor_sha256'], ref['model_sha256'] = new_desc, new_model
                updated = updated.replace(old_desc.encode(), new_desc.encode()).replace(old_model.encode(), new_model.encode())
            if updated != original:
                assert json.loads(updated) == scene, 'Hash replacement changed unrelated scene data'
                name = str(path.relative_to(library))
                replacements[name] = updated
                scenes.append(name)
        assert scenes, 'No saved scene references the repaired assets'
        replacements['3d-assets/index.json'] = json.dumps(live, separators=(',', ':')).encode() + b'\n'
        before = {}
        for name in replacements:
            path = library / name
            before[name] = path.read_bytes() if path.exists() else None
            if before[name] is not None:
                backup = output / 'before' / name
                backup.parent.mkdir(parents=True, exist_ok=True)
                backup.write_bytes(before[name])
        assert before['3d-assets/index.json'] == index_before
        report = {'scope': 'draft-asset-repair-installation-not-full-parity', 'applied': False,
                  'assets': list(edits), 'scenes': scenes, 'zipSha256': evidence['zipSha256'],
                  'files': {name: {'before': digest(before[name]) if before[name] is not None else None,
                                   'after': digest(data)} for name, data in replacements.items()}}
        (output / 'report.json').write_text(json.dumps(report, indent=2))
        if apply:
            install(library, before, replacements)
            report['applied'] = True
            (output / 'report.json').write_text(json.dumps(report, indent=2))
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', default='library')
    for name in ('stage', 'gameplay-stage', 'fixture', 'output'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--archive', help='Select a verified ZIP when the fixture retains several archives')
    args = parser.parse_args()
    print(json.dumps(publish(args.library, args.stage, args.gameplay_stage,
                             args.fixture, args.output, args.apply, args.archive), indent=2))
