"""Audit or refresh embedded editor data for already pinned asset descriptors.

This repairs the disposable local catalog only. Changed entries receive model
derivative validation; it is not a full-library publication preflight.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import tempfile

from asset_index import _relative, editor_descriptor, encoded, generate_asset_index


def refresh_index_editor(root, *, repair=False):
    if not repair:
        return _refresh_index_editor(Path(root), repair=False)
    with (Path(root) / '.publication.lock').open('a+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return _refresh_index_editor(Path(root), repair=True)


def _refresh_index_editor(root, *, repair):
    root = Path(root)
    target = root / 'index.json'
    raw = target.read_bytes()
    before_hash = hashlib.sha256(raw).hexdigest()
    index = json.loads(raw)
    del raw
    changes = []
    selected = []
    for entry in index['assets']:
        name = _relative(entry['descriptor'])
        descriptor_bytes = (root / name).read_bytes()
        if hashlib.sha256(descriptor_bytes).hexdigest() != entry['descriptor_sha256']:
            raise ValueError('Descriptor pin must be published first: ' + entry['id'])
        expected = editor_descriptor(json.loads(descriptor_bytes))
        actual = entry.get('editor', {})
        if actual == expected:
            continue
        changes.append({
            'asset': entry['id'],
            'fields': sorted(key for key in actual.keys() | expected.keys()
                             if actual.get(key) != expected.get(key)),
        })
        selected.append(name)
    report = {
        'scope': 'embedded-editor-data; changed-entry-derivatives-only',
        'assets': len(index['assets']),
        'changes': changes,
        'repaired': False,
        'beforeIndexSha256': before_hash,
    }
    if not repair or not selected:
        return report
    # Rebuild from descriptors, including derivative checks, before touching the cache.
    fresh = generate_asset_index(root, descriptors=selected)
    replacements = {entry['id']: entry for entry in fresh['assets']}
    assert set(replacements) == {change['asset'] for change in changes}
    for entry in index['assets']:
        replacement = replacements.get(entry['id'])
        if replacement is None:
            continue
        for key in ('descriptor', 'descriptor_sha256', 'model'):
            if entry[key] != replacement[key]:
                raise ValueError('Asset changed during refresh: ' + entry['id'])
        entry['editor'] = replacement['editor']
    data = encoded(index)
    mode = target.stat().st_mode & 0o777
    fd, temporary = tempfile.mkstemp(prefix='.index-editor-', suffix='.tmp', dir=root)
    try:
        with os.fdopen(fd, 'wb') as output:
            os.fchmod(output.fileno(), mode)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        with target.open('rb') as current:
            if hashlib.file_digest(current, 'sha256').hexdigest() != before_hash:
                raise ValueError('Asset index changed during refresh')
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    report['repaired'] = True
    report['afterIndexSha256'] = hashlib.sha256(data).hexdigest()
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?',
                        default=Path(__file__).resolve().parents[1] / 'library/3d-assets')
    parser.add_argument('--repair', action='store_true')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    report = refresh_index_editor(args.root, repair=args.repair)
    text = json.dumps(report, indent=2) + '\n'
    if args.report:
        args.report.write_text(text)
    print(text, end='')
