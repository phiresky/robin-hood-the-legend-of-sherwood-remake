"""Verify a frozen net preview package; install only with --install after review."""
from pathlib import Path
import argparse
import fcntl
import hashlib
import json
import os


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checked_path(base, relative):
    path = Path(relative)
    assert not path.is_absolute() and '..' not in path.parts
    assert path.parts[0] == 'mission-states'
    return base / path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--install', action='store_true')
    args = parser.parse_args()
    root = Path('level-editor/work/croisement02-refinement/restart2-state')
    stage = root / 'net-mission-package-v1'
    proof = root / 'full-editor-net-variants-v1'
    library = Path('level-editor/library')
    manifest = json.loads((stage / 'manifest.json').read_text())
    verification = json.loads((proof / 'verification.json').read_text())
    review = json.loads((proof / 'self-review.json').read_text())
    assert verification['status'] == 'PASS'
    assert len(verification['checks']) == 303
    assert all(row['pass'] for row in verification['checks'])
    assert verification['manifest_sha256'] == digest(stage / 'manifest.json')
    assert review['status'] == 'PASS'
    assert review['verification_sha256'] == digest(proof / 'verification.json')
    assert review['runtime'] == verification['runtimePins']
    for path, expected in review['runtime'].items():
        assert digest(Path(path)) == expected, path
    for path, expected in review['evidence'].items():
        assert digest(proof / path) == expected, path
    rows = manifest['files'] + [manifest['private_index']]
    assert len({row['path'] for row in rows}) == len(rows)
    assert {row['path']: row['sha256'] for row in verification['staged_hashes']} == {
        row['path']: row['sha256'] for row in rows}
    index = library / 'mission-states/index.json'
    assert digest(index) == manifest['installed_index_sha256']
    for row in rows:
        source = checked_path(stage / 'library', row['path'])
        assert digest(source) == row['sha256'], row['path']
        assert source.stat().st_size == row['bytes'], row['path']
        target = checked_path(library, row['path'])
        if target.exists() and target != index:
            assert digest(target) == row['sha256'], row['path']
    for row in manifest['reused']:
        assert digest(checked_path(library, row['path'])) == row['sha256']
    old = json.loads(index.read_text())
    new = json.loads((stage / 'library/mission-states/index.json').read_text())
    assert all(entry in new['entries'] for entry in old['entries'])
    assert len(new['entries']) == len(old['entries']) + 20
    if not args.install:
        print(json.dumps({'status': 'VERIFIED_ONLY', 'new_entries': 20,
                          'files': len(rows), 'reused': len(manifest['reused']),
                          'publication': 'Not performed; root review and slot required.'}))
        return
    root_review = json.loads((proof / 'root-review.json').read_text())
    assert 'PASS' in root_review['status']
    assert root_review['verification_sha256'] == digest(proof / 'verification.json')
    assert root_review['self_review_sha256'] == digest(proof / 'self-review.json')
    with (stage / 'install.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        assert digest(index) == manifest['installed_index_sha256']
        baseline = {str(p.relative_to(library)): digest(p)
                    for p in (library / 'mission-states').rglob('*') if p.is_file()}
        for row in manifest['files']:
            target = checked_path(library, row['path'])
            assert target != index
            source = checked_path(stage / 'library', row['path'])
            assert digest(source) == row['sha256']
            target.parent.mkdir(parents=True, exist_ok=True)
            if not target.exists():
                with target.open('xb') as output:
                    output.write(source.read_bytes())
                    output.flush()
                    os.fsync(output.fileno())
            assert digest(target) == row['sha256']
        # Validate all staged and reused bytes before the single catalog switch.
        for row in manifest['files'] + manifest['reused']:
            assert digest(checked_path(library, row['path'])) == row['sha256']
        assert digest(index) == manifest['installed_index_sha256']
        pending = index.with_name('index.net-variants-pending.json')
        with pending.open('xb') as output:
            output.write((stage / 'library/mission-states/index.json').read_bytes())
            output.flush()
            os.fsync(output.fileno())
        assert digest(pending) == manifest['private_index']['sha256']
        os.replace(pending, index)
        for path, expected in baseline.items():
            if path != 'mission-states/index.json':
                assert digest(library / path) == expected
        receipt = {'status': 'INSTALLED', 'entries_added': 20,
                   'files_added_or_preserved': len(manifest['files']),
                   'baseline_nonindex_preserved': True, 'baseline_files': len(baseline),
                   'index_sha256': digest(index),
                   'manifest_sha256': digest(stage / 'manifest.json'),
                   'verification_sha256': digest(proof / 'verification.json'),
                   'root_review_sha256': digest(proof / 'root-review.json'),
                   'scope': 'Mission preview catalog only; no static map publication.'}
        (stage / 'installation.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print(json.dumps(receipt))


if __name__ == '__main__':
    main()
