"""Install browser-verified asset repairs, derivatives and saved-scene pins together."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys
import zipfile

from asset_index import validate_asset_index
from promote_staged_publication import asset_file_pairs, safe_relative
from publish_library import gameplay_frame_glb

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from lossy_assets import LibraryLock, atomic_write, verify_derivatives


def digest(data):
    return hashlib.sha256(data).hexdigest()


def required_derivatives(entry, model):
    fields = ('lossy_model', 'preview_model')
    if all(entry.get(field) for field in fields):
        return fields
    assert not any(entry.get(field) for field in fields), 'Incomplete derivative pair: ' + entry['id']
    # Empty gameplay frames have no geometry or textures to derive. Apply the
    # same strict GLB/resource validation used when packaging the library.
    gameplay_frame_glb(model, entry)
    return ()


def verify_descriptor_edit(previous, replacement, edit):
    """Allow only the gameplay and explicitly listed part appearance changes."""
    expected = copy.deepcopy(previous)
    expected['gameplay'] = edit['gameplay']
    for node, appearance in edit.get('partAppearances', {}).items():
        parts = [part for part in expected['parts'] if part['node'] == node]
        assert len(parts) == 1, 'Appearance edit requires one existing part: ' + node
        parts[0]['appearance'] = appearance
    assert replacement == expected, 'Descriptor changed outside the reviewed gameplay/appearances'


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


def add_reviewed_connections(original, review, edits):
    """Install reviewed links while preserving unrelated scene bytes."""
    assert digest(original) == review['sceneSha256'], 'Stale connection scene'
    scene = json.loads(original)
    existing = scene.get('interiorConnections', [])
    connections = review['interiorConnections']
    assert isinstance(connections, list) and connections, 'Missing reviewed connections'
    migrations = review.get('migrated', [])
    if migrations:
        assert review['beforeLinks'] == existing, 'Stale migration connections'
        expected = copy.deepcopy(existing)
        touched = set()
        for migration in migrations:
            identity, side = migration['id'], migration['side']
            assert side in ('from', 'to'), 'Invalid migration side'
            assert (identity, side) not in touched, 'Duplicate endpoint migration'
            touched.add((identity, side))
            links = [link for link in expected if link['id'] == identity]
            assert len(links) == 1, 'Missing migration connection'
            before, after = migration['before'], migration['after']
            assert links[0][side] == before, 'Stale migration endpoint'
            assert before != after, 'Empty endpoint migration'
            assert all(endpoint['asset'] in edits for endpoint in (before, after)), \
                'Migration assets must both be reviewed'
            assert not any(room['id'] == before['interior']
                           for room in edits[before['asset']]['gameplay']['interiors']), \
                'Migrated source room must be removed'
            links[0][side] = after
        assert connections == expected, 'Unreviewed connection migration'
    else:
        assert connections[:len(existing)] == existing, 'Do not change existing scene connections'
        assert len(connections) > len(existing), 'No new room connections'
    placements = {placement['id']: placement for placement in scene['placements']}
    ids, pairs = set(), set()
    for index, connection in enumerate(connections):
        identity = connection['id']
        assert isinstance(identity, str) and identity and identity not in ids, 'Duplicate connection ID'
        ids.add(identity)
        endpoints = []
        for side in ('from', 'to'):
            endpoint = connection[side]
            asset, placement, interior = (endpoint[key] for key in ('asset', 'placement', 'interior'))
            assert placement in placements and asset in placements[placement]['assets'], 'Missing connection placement'
            assert asset in edits or index < len(existing), 'Connection asset is not in the reviewed publication'
            if asset in edits:
                assert any(room['id'] == interior for room in edits[asset]['gameplay']['interiors']), 'Missing connection room'
            endpoints.append((placement, asset, interior))
        assert endpoints[0][:2] != endpoints[1][:2], 'Local room links belong to the asset'
        pair = tuple(sorted(endpoints))
        assert pair not in pairs, 'Duplicate room connection'
        pairs.add(pair)
    scene['interiorConnections'] = connections
    text = original.decode().rstrip()
    assert text.endswith('}')
    encoded = json.dumps(connections, indent=2).replace('\n', '\n  ')
    if 'interiorConnections' in json.loads(original):
        # Decode root fields to locate the value without rewriting unrelated numbers.
        decoder = json.JSONDecoder()
        cursor = text.index('{') + 1
        while True:
            while text[cursor].isspace():
                cursor += 1
            key, cursor = decoder.raw_decode(text, cursor)
            while text[cursor].isspace():
                cursor += 1
            assert text[cursor] == ':'
            cursor += 1
            while text[cursor].isspace():
                cursor += 1
            start = cursor
            value, cursor = decoder.raw_decode(text, cursor)
            if key == 'interiorConnections':
                assert value == existing
                appended = (text[:start] + encoded + text[cursor:] + '\n').encode()
                break
            while text[cursor].isspace():
                cursor += 1
            assert text[cursor] == ',', 'Missing root connection field'
            cursor += 1
    else:
        appended = (text[:-1].rstrip() + ',\n  "interiorConnections": ' + encoded + '\n}\n').encode()
    assert json.loads(appended) == scene, 'Connection insertion changed unrelated scene data'
    return appended


def publish(library, stage, gameplay_stage, fixture, output, apply=False, archive_name=None,
            allow_unreferenced=False, scene_connections=None):
    library, stage, gameplay_stage, fixture, output = map(
        lambda value: Path(value).resolve(), (library, stage, gameplay_stage, fixture, output))
    assert not output.is_relative_to(library)
    output.mkdir(parents=True, exist_ok=False)
    staged_root = stage / '3d-assets'
    index = json.loads((staged_root / 'index.json').read_bytes())
    entries = index['assets']
    edits = {edit['asset']: edit for edit in json.loads((gameplay_stage / 'edits.json').read_bytes())}
    connection_review = json.loads(Path(scene_connections).read_bytes()) if scene_connections else None
    connection_scene = Path(connection_review['scenePath']).resolve() if connection_review else None
    if connection_scene:
        assert connection_scene.parent == library / 'scenes' and connection_scene.is_file()
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
            verify_descriptor_edit(old_descriptor, new_descriptor, edit)
            assert new_descriptor['gameplay'].get('draft', {}).get('issues'), 'Retain draft limitations'
            ref = references[asset]
            assert ref['descriptor_sha256'] == digest(new) and ref['model_sha256'] == digest(model)
            assert (fixture / ref['descriptor']).read_bytes() == new
            assert (fixture / ref['model']).read_bytes() == model
            for source, target in asset_file_pairs(staged_root, root, entry):
                replacements[str(target.relative_to(library))] = source.read_bytes()
            derivative_fields = required_derivatives(entry, model)
            if not derivative_fields:
                assert not current.get('lossy_model') and not current.get('preview_model'), \
                    'Cannot drop existing derivatives: ' + asset
            for field in derivative_fields:
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
            updated = (add_reviewed_connections(original, connection_review, edits)
                       if path == connection_scene else original)
            scene = json.loads(updated)
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
        assert scenes or allow_unreferenced, 'No saved scene references the repaired assets'
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
    parser.add_argument('--allow-unreferenced', action='store_true',
                        help='Publish library drafts not yet referenced by saved scenes; all asset/archive checks still apply')
    parser.add_argument('--scene-connections', help='Reviewed scene hash and new interior connections to install atomically')
    args = parser.parse_args()
    print(json.dumps(publish(args.library, args.stage, args.gameplay_stage,
                             args.fixture, args.output, args.apply, args.archive,
                             args.allow_unreferenced, args.scene_connections), indent=2))
