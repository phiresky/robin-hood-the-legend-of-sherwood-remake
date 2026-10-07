"""Bind the installed HTTP Editor check to unchanged published files and code."""
import hashlib
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire, release

WORK = ROOT / 'level-editor/work/croisement02-refinement/restart2-textures'
STAGE = WORK / 'post-batch15-static-candidate-v6'
FIXTURE = WORK / 'installed-static-v1'
RUNNER = WORK / 'review_installed_static_v1.mjs'
SUPPLEMENT = None


def sha(path):
    return hashlib.file_digest(path.open('rb'), 'sha256').hexdigest() if path.is_file() else None


def snapshot():
    manifest_path = STAGE / 'promotion.json'
    manifest = json.loads(manifest_path.read_text())
    if manifest['status'] != 'APPLIED':
        raise ValueError('Static publication is not applied')
    pins = {str(manifest_path): sha(manifest_path)}
    replacements = {}
    if SUPPLEMENT is not None:
        supplement = json.loads(SUPPLEMENT.read_text())
        if supplement['manifest_sha256'] != sha(manifest_path):
            raise ValueError('Supplement belongs to another publication')
        pins[str(SUPPLEMENT)] = sha(SUPPLEMENT)
        for name, digest in supplement['evidence'].items():
            if sha(Path(name)) != digest:
                raise ValueError('Supplement evidence changed: ' + name)
            pins[name] = digest
        replacements = supplement['files']
        if not set(replacements).issubset({row['target'] for row in manifest['files']}):
            raise ValueError('Supplement contains unknown publication targets')
    for row in manifest['files']:
        path = Path(row['target'])
        expected = row['source_sha256'] if row['source'] is not None else None
        if str(path) in replacements:
            replacement = replacements[str(path)]
            if replacement['previous_sha256'] != expected:
                raise ValueError('Supplement baseline mismatch: ' + str(path))
            expected = replacement['sha256']
        if str(path) == manifest['index_generation']['target']:
            # Other maps can publish after this map's transaction. Its own
            # catalog entries must remain exact; pin the full current index.
            before = json.loads(Path(row['source']).read_text())
            after = json.loads(path.read_text())
            for entry in before['assets']:
                descriptor = path.parent / entry['descriptor']
                replacement = replacements.get(str(descriptor))
                if replacement is not None:
                    if entry['descriptor_sha256'] != replacement['previous_sha256']:
                        raise ValueError('Catalog supplement baseline mismatch')
                    if sha(descriptor) != replacement['sha256']:
                        raise ValueError('Catalog supplement descriptor changed')
                    entry['descriptor_sha256'] = replacement['sha256']
                    entry['editor'] = json.loads(descriptor.read_text())
            def scoped(index):
                return [entry for entry in index['assets']
                        if entry.get('source_map', '').lower() == 'croisement02']
            if before['version'] != after['version'] or scoped(before) != scoped(after):
                raise ValueError('Installed Crossings02 catalog changed')
            expected = sha(path)
        if sha(path) != expected:
            raise ValueError('Installed publication changed: ' + str(path))
        pins[str(path)] = expected
    for row in manifest['protected_files']:
        path = Path(row['path'])
        if sha(path) != row['sha256']:
            raise ValueError('Protected publication changed: ' + str(path))
        pins[str(path)] = row['sha256']
    for directory in ('app/src', 'shared/src'):
        for path in (ROOT / 'level-editor' / directory).rglob('*'):
            if path.is_file():
                pins[str(path)] = sha(path)
    for path in (RUNNER, FIXTURE / 'editor-review.html', FIXTURE / 'editor-review.tsx',
                 ROOT / 'level-editor/app/vite.config.ts'):
        pins[str(path)] = sha(path)
    return pins


def main():
    global FIXTURE, RUNNER, SUPPLEMENT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path, default=FIXTURE)
    parser.add_argument('--runner', type=Path, default=RUNNER)
    parser.add_argument('--supplement', type=Path)
    args = parser.parse_args()
    FIXTURE, RUNNER = args.fixture.resolve(), args.runner.resolve()
    SUPPLEMENT = args.supplement.resolve() if args.supplement else None
    acquire()
    try:
        output = FIXTURE / 'runtime'
        if output.exists() and any(output.iterdir()):
            raise ValueError('Retain existing browser evidence; choose a fresh fixture for a retry')
        pins = snapshot()
        (FIXTURE / 'prelaunch-pins.json').write_text(json.dumps(pins, indent=2) + '\n')
        subprocess.run(['node', str(RUNNER)], cwd=ROOT, check=True)
        if snapshot() != pins:
            raise ValueError('Inputs changed during installed-library check')
        result_path = output / 'result.json'
        result = json.loads(result_path.read_text())
        if not result['status'].startswith('PASS'):
            raise ValueError('Installed browser proof did not pass')
        map_path = ROOT / 'level-editor/library/scenes/croisement02.rhlos-map.json'
        proof = {
            'status': 'PASS',
            'scope': 'Installed production Editor via normal HTTP library, without resource interception',
            'map_sha256': sha(map_path),
            'inputs': pins,
            'result': str(result_path),
            'result_sha256': sha(result_path),
            'screenshots': {name: sha(output / name) for name in ('loaded.png', 'rotated.png')},
            'visual_review': 'Pending independent root inspection',
        }
        (FIXTURE / 'verification.json').write_text(json.dumps(proof, indent=2) + '\n')
        print('PASS installed normal HTTP proof; exact inputs unchanged')
    finally:
        release()


if __name__ == '__main__':
    main()
