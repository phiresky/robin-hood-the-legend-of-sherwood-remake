"""Verify and install the explicitly approved York grouping milestone.

This preserves the reviewed models and their unfinished materials. It does not
claim completed geometry, texture refinement or game-state reconstruction.
Only York's local library directory, map and the shared index are replaced.
"""
import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

EDITOR = Path(__file__).resolve().parents[2]
WORK = EDITOR / 'work/active/york'
LIBRARY = EDITOR / 'library'
sys.path[:0] = [str(EDITOR/'refinement'), str(EDITOR/'refinement/blender')]
from asset_index import discover_asset_index, write_asset_index
from lossy_assets import verify_derivatives
from promote_staged_publication import library_lock
from scene_manifest import scene_metadata
from stored_map import asset_source_references


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2)+'\n')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def tree_hashes(root):
    return {p.relative_to(root).as_posix(): sha(p) for p in sorted(root.rglob('*')) if p.is_file()}


def verify(stage, *, browser=False):
    stage = Path(stage).resolve()
    source = WORK/'stage/map-assets'
    staged = stage/'map-assets'
    candidates = read(WORK/'review/candidates.json')['items']
    require(len(candidates) == 253 and all(c.get('user_approval') == 'approved grouping'
            for c in candidates), 'Every grouping must have its recorded explicit approval')
    require(not read(WORK/'review/evidence.json')['items'], 'Grouping review is still pending')
    expected = {c['id'] for c in candidates}
    reviewed = {c['id']:read(c['ownership']) for c in candidates}
    index = discover_asset_index(staged/'3d-assets')
    require({a['id'] for a in index['assets']} == expected, 'Staged asset membership changed')
    pins = {}
    for entry in index['assets']:
        for key in ('model', 'descriptor'):
            relative = '3d-assets/'+entry[key]
            require(sha(staged/relative) == sha(source/relative), 'Reviewed export changed: '+relative)
            require(sha(staged/relative) == reviewed[entry['id']][key+'_sha256'],
                    'Export differs from approved review evidence: '+relative)
            pins[relative] = sha(staged/relative)
        descriptor = read(staged/'3d-assets'/entry['descriptor'])
        for resource in descriptor.get('resources', []):
            relative = resource['path']
            require(sha(staged/relative) == resource['sha256'], 'Resource pin changed: '+relative)
            pins[relative] = resource['sha256']
        for key in ('lossy_model', 'preview_model'):
            require(entry.get(key), 'Missing browser derivative: '+entry['id']+' '+key)
            relative = '3d-assets/'+entry[key]
            pins[relative] = sha(staged/relative)
            pins[relative+'.receipt.json'] = sha(staged/(relative+'.receipt.json'))
    require(not verify_derivatives(staged/'3d-assets'), 'Browser derivative verification failed')
    prior = read(WORK/'baseline/published-map.json')
    current = read(LIBRARY/'scenes/york.rhlos-map.json')
    require(current == prior, 'Live York map changed since the reviewed baseline')
    for relative, digest in read(stage/'live-before.json').items():
        require(sha(LIBRARY/relative) == digest, 'Live York file changed: '+relative)
    document_path = stage/'browser-document.rhlos-map.json'
    if not document_path.exists():
        document_path = stage/'york.rhlos-map.json'
    document = read(document_path)
    for key in ('map', 'sourceMap', 'size', 'camera', 'sceneMetadata', 'provenance'):
        require(document.get(key) == prior.get(key), 'Map metadata changed: '+key)
    require(document['placements'] == read(WORK/'stage/york.rhlos-map.json')['placements'],
            'Reviewed world placements changed')
    scene_metadata(staged, document)
    require(len(document['placements']) == 252, 'Unexpected placement count')
    old_ids = {a['id'] for a in discover_asset_index(LIBRARY/'3d-assets/york')['assets']}
    for path in (LIBRARY/'scenes').glob('*.rhlos-map.json'):
        if path.name == 'york.rhlos-map.json':
            continue
        other = read(path)
        references = other.get('sceneAssets', []) + list(asset_source_references(other))
        require(not any(r['id'] in old_ids for r in references),
                'Another map references retiring York assets: '+str(path))
    result = {'status':'PASS', 'scope':'approved grouping milestone', 'assets':len(expected),
              'reviewed_models_and_descriptors_identical':True, 'placements_preserved':True,
              'map_metadata_preserved':True, 'document':str(document_path),
              'document_sha256':sha(document_path), 'files':pins,
              'catalog_sha256':sha(EDITOR/'refinement/catalogs/york.json'),
              'decisions_sha256':sha(WORK/'grouping-decisions.json')}
    if browser:
        config_path = stage/'browser/config.json'
        audit = read(stage/'browser/result.json')
        require(audit.get('status') == 'PASS' and not audit.get('visualOnly'), 'Full editor audit required')
        config = read(config_path)
        require(config['mode'] == 'staged' and config['map'] == 'york', 'Wrong browser audit scope')
        browser_files = {r['path']:r['sha256'] for r in config['files']}
        require(browser_files.get('scenes/york.rhlos-map.json') == result['document_sha256'],
                'Browser audited a different map document')
        for relative, digest in pins.items():
            if not relative.endswith('.receipt.json') and not relative.endswith('/asset.json'):
                require(browser_files.get(relative) == digest, 'Browser omitted staged payload: '+relative)
        for record in config['files']:
            require(record['url'].startswith('/@fs/'), 'Unexpected browser input URL')
            require(sha(Path(record['url'][5:])) == record['sha256'], 'Browser input changed: '+record['path'])
        require(set(config['expected']['base_asset_ids']) == expected, 'Browser asset coverage changed')
        result['browser_config_sha256'] = sha(config_path)
        result['browser_result_sha256'] = sha(stage/'browser/result.json')
    write(stage/'installation-verification.json', result)
    print(json.dumps({k:v for k,v in result.items() if k!='files'}), flush=True)
    return result


def install(stage):
    stage = Path(stage).resolve()
    with library_lock(LIBRARY):
        proof = verify(stage, browser=True)
        backup = stage/'installation-backup'
        backup.mkdir(exist_ok=False)
        target = LIBRARY/'3d-assets/york'
        incoming = LIBRARY/'3d-assets/.york-grouping-incoming'
        require(not incoming.exists(), 'Incoming directory already exists')
        map_path = LIBRARY/'scenes/york.rhlos-map.json'
        index_path = LIBRARY/'3d-assets/index.json'
        shutil.copy2(map_path, backup/'york.rhlos-map.json')
        shutil.copy2(index_path, backup/'index.json')
        shared_added = []
        retired = False
        try:
            shutil.copytree(stage/'map-assets/3d-assets/york', incoming)
            for relative, digest in proof['files'].items():
                if relative.startswith('3d-assets/york/'):
                    continue
                destination = (LIBRARY/relative).resolve()
                require(destination.is_relative_to((LIBRARY/'3d-assets').resolve()), 'Resource escaped asset library')
                if destination.exists():
                    require(sha(destination) == digest, 'Shared resource conflicts: '+relative)
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shared_added.append(relative)
                    shutil.copy2(stage/'map-assets'/relative, destination)
            target.rename(backup/'york')
            retired = True
            incoming.rename(target)
            temporary = map_path.with_name('york.grouping-install.tmp')
            require(not temporary.exists(), 'Temporary map already exists')
            shutil.copy2(proof['document'], temporary)
            temporary.replace(map_path)
            write_asset_index(LIBRARY/'3d-assets')
            for relative, digest in proof['files'].items():
                require(sha(LIBRARY/relative) == digest, 'Installed payload differs from verification: '+relative)
            require(tree_hashes(target) == tree_hashes(stage/'map-assets/3d-assets/york'), 'Installed York bytes differ')
            scene_metadata(LIBRARY, read(map_path))
            require(sha(map_path) == proof['document_sha256'], 'Installed map differs')
        except BaseException:
            if retired:
                if target.exists():
                    shutil.rmtree(target)
                (backup/'york').rename(target)
            if incoming.exists():
                shutil.rmtree(incoming)
            for relative in shared_added:
                (LIBRARY/relative).unlink(missing_ok=True)
            shutil.copy2(backup/'york.rhlos-map.json', map_path)
            shutil.copy2(backup/'index.json', index_path)
            raise
        receipt = {'status':'APPLIED', 'scope':'grouping-only library update',
                   'assets':253, 'map_sha256':sha(map_path), 'installed_files':tree_hashes(target),
                   'shared_resources_added':shared_added, 'backup':str(backup),
                   'verification_sha256':sha(stage/'installation-verification.json')}
        write(stage/'installation.json', receipt)
        print('Installed 252 named York assets plus terrain; rollback backup: '+str(backup))


def rollback(stage):
    stage = Path(stage).resolve()
    with library_lock(LIBRARY):
        receipt = read(stage/'installation.json')
        require(receipt['status'] == 'APPLIED', 'Installation is not active')
        target = LIBRARY/'3d-assets/york'
        map_path = LIBRARY/'scenes/york.rhlos-map.json'
        require(tree_hashes(target) == receipt['installed_files'], 'York changed since installation')
        require(sha(map_path) == receipt['map_sha256'], 'York map changed since installation')
        backup = Path(receipt['backup'])
        target.rename(stage/'rolled-back-york')
        (backup/'york').rename(target)
        shutil.copy2(backup/'york.rhlos-map.json', map_path)
        # Rebuild, rather than overwrite other maps' subsequent index changes.
        write_asset_index(LIBRARY/'3d-assets')
        receipt['status'] = 'ROLLED_BACK'
        write(stage/'installation.json', receipt)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('verify', 'install', 'rollback'))
    parser.add_argument('stage', type=Path)
    parser.add_argument('--browser', action='store_true')
    args = parser.parse_args()
    if args.action == 'verify':
        verify(args.stage, browser=args.browser)
    elif args.action == 'install':
        install(args.stage)
    else:
        rollback(args.stage)
