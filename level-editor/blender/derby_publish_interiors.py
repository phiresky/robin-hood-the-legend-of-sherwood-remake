"""Stage or install Derby room repairs with pinned inputs and rollback.

The full publication browser audit must pass against the exact staged files.
Use stage <library> <state-output> <publication>, then prepare_publication_browser.py
with --map derby --document <publication>/derby.rhlos-map.json and scope.json.
Use apply <library> <publication> only after inspecting the captured states.
"""
import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve()
sys.path[:0] = [str(HERE.parents[1] / 'refinement'), str(HERE.parents[1] / 'refinement/blender')]
from asset_index import write_asset_index, editor_descriptor
from lossy_assets import verify_derivatives
from promote_staged_publication import library_lock
from promote_state_bundles import atomic

PATCHES = {'derby-keep-west-tower': 'patch-001', 'derby-east-hall': 'patch-002',
           'derby-upper-gatehouse': 'patch-003', 'derby-keep-main-hall': 'patch-000'}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + '\n')


def stage(library, states, output):
    library, states, output = [Path(p).resolve() for p in (library, states, output)]
    integration = json.loads((states / 'integration.json').read_text())
    selected = set(integration['outputs'])
    if not selected or not selected <= PATCHES.keys() or not integration['covered_resources_unchanged']:
        raise ValueError('Incomplete interior-state verification')
    output.mkdir(parents=True, exist_ok=False)
    target = output / 'map-assets'
    for asset in (library / '3d-assets/derby').iterdir():
        if not asset.is_dir():
            continue
        dst = target / '3d-assets/derby' / asset.name
        dst.mkdir(parents=True)
        if asset.name in selected:
            for name, key in [('model.glb', 'model_sha256'), ('asset.json', 'descriptor_sha256')]:
                if sha(states / asset.name / name) != integration['outputs'][asset.name][key]:
                    raise ValueError('State output changed: ' + asset.name)
                shutil.copy2(states / asset.name / name, dst / name)
        else:
            for path in asset.iterdir():
                if path.is_file():
                    os.link(path, dst / path.name)
    write_asset_index(target / '3d-assets')
    subprocess.run(['blender', '--background', '--threads', '2', '--python-exit-code', '1',
                    '--python', str(HERE.parents[1] / 'refinement/blender/lossy_assets.py'),
                    '--', 'refresh', '--root', str(target / '3d-assets'),
                    '--work', str(output / 'derivatives'), '--assets', *sorted(selected)], check=True)
    scene_path = library / 'scenes/derby.rhlos-map.json'
    scene = json.loads(scene_path.read_text())
    for ref in scene['assetSources']:
        if ref['id'] in selected:
            ref['model_sha256'] = sha(target / ref['model'])
            ref['descriptor_sha256'] = sha(target / ref['descriptor'])
    found = set()
    for placement in scene['placements']:
        if placement['id'] in selected:
            asset = placement['id']
            placement.setdefault('patches', {})[asset] = {'appearance-1': PATCHES[asset]}
            found.add(asset)
    if found != selected:
        raise ValueError('Missing room placement')
    write(output / 'derby.rhlos-map.json', scene)
    (target / 'scenes').mkdir()
    write(target / 'scenes/derby.rhlos-map.json', scene)
    write(output / 'scope.json', {
        'asset_ids': sorted(selected),
        'already_published': sorted({r['id'] for r in scene['assetSources']} - selected),
        'required_patches': sorted(set(PATCHES.values())),
    })
    files = {}
    for asset in selected:
        for name in ('model.glb', 'asset.json', 'preview.glb', 'preview.glb.receipt.json'):
            relative = '3d-assets/derby/' + asset + '/' + name
            files[relative] = {'before': sha(library / relative), 'after': sha(target / relative)}
    relative = 'scenes/derby.rhlos-map.json'
    files[relative] = {'before': sha(scene_path), 'after': sha(target / relative)}
    removals = {}
    for asset in selected:
        for name in ('lossy.glb', 'lossy.glb.receipt.json'):
            relative = '3d-assets/derby/' + asset + '/' + name
            if (library / relative).exists():
                removals[relative] = sha(library / relative)
    write(output / 'install-plan.json', {'version': 1, 'inputs': integration['inputs'],
                                        'assets': sorted(selected), 'files': files,
                                        'removals': removals})


def apply(library, output):
    library, output = [Path(p).resolve() for p in (library, output)]
    plan = json.loads((output / 'install-plan.json').read_text())
    selected = set(plan['assets'])
    if not selected or not selected <= PATCHES.keys():
        raise ValueError('Unknown interior repair assets')
    result = json.loads((output / 'browser/result.json').read_text())
    if result.get('status') != 'PASS' or result.get('visualOnly'):
        raise ValueError('Full browser audit has not passed')
    config = json.loads((output / 'browser/config.json').read_text())
    if set(config['expected']['required_patches']) != {'patch-000', *PATCHES.values()}:
        raise ValueError('Audit omitted an interior state')
    target = output / 'map-assets'
    pinned = {f['path']: f['sha256'] for f in config['files']}
    # The HTTP catalog embeds descriptors, so they need not be served as
    # separate files. Validate both the catalog pin and its embedded content.
    index_path = output / 'browser/private-index.json'
    if sha(index_path) != pinned.get('3d-assets/index.json'):
        raise ValueError('Audited catalog changed after verification')
    for entry in json.loads(index_path.read_text())['assets']:
        if entry['id'] not in selected:
            continue
        relative = '3d-assets/' + entry['descriptor']
        if editor_descriptor(json.loads((target / relative).read_text())) != entry['editor']:
            raise ValueError('Audited descriptor content differs: ' + relative)
        pinned[relative] = entry['descriptor_sha256']
    problems = verify_derivatives(target / '3d-assets')
    if problems:
        raise ValueError('Staged derivatives are inconsistent: ' + repr(problems))
    # Derivatives may be refreshed after lossless staging. Their receipts must
    # bind the models, and the browser must have loaded these exact new bytes.
    for asset in selected:
        for name in ('lossy.glb', 'lossy.glb.receipt.json', 'preview.glb', 'preview.glb.receipt.json'):
            relative = '3d-assets/derby/' + asset + '/' + name
            if not (target / relative).is_file():
                continue
            if relative not in plan['files']:
                if relative not in plan['removals']:
                    raise ValueError('Derivative has no pinned live predecessor: ' + relative)
                plan['files'][relative] = {'before': plan['removals'].pop(relative)}
            plan['files'][relative]['after'] = sha(target / relative)
    # The browser uses the format bridge's normalized serialization. Publish
    # those exact tested bytes, accepting no semantic change to the staged map.
    relative = 'scenes/derby.rhlos-map.json'
    audited = output / 'browser-document.rhlos-map.json'
    if json.loads(audited.read_text()) != json.loads((target / relative).read_text()):
        raise ValueError('Audited map differs from the staged placements')
    if sha(audited) != pinned.get(relative):
        raise ValueError('Audited map changed after verification')
    shutil.copy2(audited, target / relative)
    plan['files'][relative]['after'] = sha(audited)
    for relative, entry in plan['files'].items():
        if relative.endswith('.receipt.json'):
            continue
        if sha(target / relative) != entry['after'] or pinned.get(relative) != entry['after']:
            raise ValueError('Audited output differs: ' + relative)
    backup = output / 'backup'
    with ExitStack() as locks:
        locks.enter_context(library_lock(library))
        locks.enter_context(library_lock(library / '3d-assets'))
        for path, expected in plan['inputs'].items():
            if sha(path) != expected:
                raise ValueError('Repair input changed: ' + path)
        before = {key: entry['before'] for key, entry in plan['files'].items()}
        before.update(plan['removals'])
        for relative, expected in before.items():
            if sha(library / relative) != expected:
                raise ValueError('Live input changed: ' + relative)
        for relative, entry in plan['files'].items():
            if sha(target / relative) != entry['after']:
                raise ValueError('Staged file changed: ' + relative)
        backup.mkdir(exist_ok=False)
        before['3d-assets/index.json'] = sha(library / '3d-assets/index.json')
        for relative in before:
            dst = backup / relative
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(library / relative, dst)
        try:
            for relative in plan['files']:
                atomic(library / relative, (target / relative).read_bytes())
            for relative in plan['removals']:
                (library / relative).unlink()
            write_asset_index(library / '3d-assets')
            for relative, entry in plan['files'].items():
                if sha(library / relative) != entry['after']:
                    raise ValueError('Installed bytes differ: ' + relative)
        except BaseException:
            for relative in before:
                atomic(library / relative, (backup / relative).read_bytes())
            raise
        receipt = {'status': 'PASS', 'assets': sorted(selected), 'backup': str(backup),
                   'files': {key: sha(library / key) for key in plan['files']},
                   'browser_result_sha256': sha(output / 'browser/result.json')}
        write(output / 'installed.json', receipt)
        return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    prepare = commands.add_parser('stage')
    prepare.add_argument('library', type=Path)
    prepare.add_argument('states', type=Path)
    prepare.add_argument('output', type=Path)
    install = commands.add_parser('apply')
    install.add_argument('library', type=Path)
    install.add_argument('output', type=Path)
    args = parser.parse_args()
    if args.command == 'stage':
        stage(args.library, args.states, args.output)
    else:
        print(json.dumps(apply(args.library, args.output), indent=2))
