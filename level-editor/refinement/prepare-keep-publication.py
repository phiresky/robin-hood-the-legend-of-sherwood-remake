"""Prepare a guarded publication of the assembled keep and its verified derivatives.

Run from level-editor. Apply the resulting promotion.json with
promote_staged_publication.py --apply; preparation never edits the live library.
"""
import argparse
import json
from pathlib import Path
import zipfile

from asset_index import validate_asset_index, write_asset_index
from promote_staged_publication import asset_file_pairs, library_lock, sha, verify_derivatives


def prepare(stage, fixture, output, library):
    stage, fixture, output, library = (Path(p).resolve() for p in (stage, fixture, output, library))
    assert not output.is_relative_to(library)
    output.mkdir(parents=True, exist_ok=False)
    root = library / '3d-assets'
    staged = stage / '3d-assets'
    index = json.loads((staged / 'index.json').read_text())
    assert len(index['assets']) == 1
    entry = index['assets'][0]
    assert entry['id'] == 'derby-great-keep'
    validate_asset_index(staged, index)
    assert not verify_derivatives(staged)
    authoring = json.loads((stage / 'authoring.json').read_text())
    gameplay = json.loads((Path(authoring['candidate']) / 'authoring.json').read_text())
    assert sha(staged / entry['model']) == authoring['modelSha256']
    descriptor = json.loads((staged / entry['descriptor']).read_text())
    assert descriptor['gameplay']['draft']['issues'], 'Keep remaining limitations visible'
    assert len(authoring['verifiedPartSignatures']) == len(descriptor['parts']) == 71
    evidence = json.loads((fixture / 'browser-roundtrip-evidence.json').read_text())
    assert evidence['browserRoundtrip'].startswith('PASS ')
    archives = list(fixture.glob('*.zip'))
    assert len(archives) == 1 and sha(archives[0]) == evidence['zipSha256']
    assert archives[0].stat().st_size == evidence['bytes']
    with zipfile.ZipFile(archives[0]) as archive:
        documents = [n for n in archive.namelist() if n.startswith('editor/') and n.endswith('.rhlos-map.json')]
        assert len(documents) == 1
        document = json.loads(archive.read(documents[0]))
    reference, = [ref for ref in document['assetSources'] if ref['id'] == entry['id']]
    assert reference['descriptor_sha256'] == sha(staged / entry['descriptor'])
    assert reference['model_sha256'] == sha(staged / entry['model'])
    with library_lock(library):
        assert sha(root / entry['descriptor']) == gameplay['descriptorSha256']
        previous = json.loads((root / entry['descriptor']).read_text())
        allowed = {'gameplay', 'bounds_local_scene'}
        assert {k: v for k, v in previous.items() if k not in allowed} == {
            k: v for k, v in descriptor.items() if k not in allowed}
        protected = []
        for source in authoring['sources']:
            for filename, key in [('asset.json', 'descriptorSha256'), ('model.glb', 'modelSha256')]:
                path = root / 'derby' / source['id'] / filename
                assert sha(path) == source[key], 'Component changed since assembly'
                protected.append({'path': str(path), 'sha256': sha(path)})
        pairs = asset_file_pairs(staged, root, entry)
        for key in ('lossy_model', 'preview_model'):
            assert entry.get(key), f'Missing {key}'
            for suffix in ('', '.receipt.json'):
                relative = entry[key] + suffix
                pairs.append((staged / relative, root / relative))
        old_desc, old_model = sha(root / entry['descriptor']), sha(root / entry['model'])
        scenes = []
        for path in sorted((library / 'scenes').glob('*.rhlos-map.json')):
            data = path.read_bytes()
            scene = json.loads(data)
            changed = False
            for ref in scene.get('assetSources', []) + scene.get('sceneAssets', []):
                if ref.get('id') != entry['id']:
                    continue
                assert ref['descriptor'] == '3d-assets/' + entry['descriptor']
                assert ref['model'] == '3d-assets/' + entry['model']
                assert (ref['descriptor_sha256'], ref['model_sha256']) == (old_desc, old_model)
                ref['descriptor_sha256'] = reference['descriptor_sha256']
                ref['model_sha256'] = reference['model_sha256']
                changed = True
            if changed:
                updated = data.replace(old_desc.encode(), reference['descriptor_sha256'].encode())
                updated = updated.replace(old_model.encode(), reference['model_sha256'].encode())
                assert json.loads(updated) == scene
                source = output / path.name
                source.write_bytes(updated)
                pairs.append((source, path))
                scenes.append(path.name)
        merged = output / 'index.json'
        prospective = {str(target.relative_to(root)): source for source, target in pairs
                       if target.is_relative_to(root)}
        write_asset_index(root, target=merged, files=prospective)
        pairs.append((merged, root / 'index.json'))
        records = [{'source': str(source), 'target': str(target), 'source_sha256': sha(source),
                    'previous_sha256': sha(target), 'backup': str(output / 'before' / str(i))}
                   for i, (source, target) in enumerate(pairs)]
        manifest = {'status': 'PREPARED_NOT_APPLIED', 'library': str(library), 'files': records,
                    'protected_files': protected, 'index_generation': {'target': str(root / 'index.json')},
                    'scope': 'usable-draft-assembly-not-full-parity', 'scenes': scenes,
                    'browser_check': evidence, 'limitations': descriptor['gameplay']['draft']['issues']}
        (output / 'promotion.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(output / 'promotion.json')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', required=True)
    parser.add_argument('--fixture', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--library', default='library')
    args = parser.parse_args()
    prepare(args.stage, args.fixture, args.output, args.library)
