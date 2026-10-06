"""Install a reviewed incremental mission preview with an atomic catalog switch."""
from pathlib import Path
import fcntl
import hashlib
import json
import os

root = Path('level-editor/work/croisement02-refinement/restart2-state')
stage = root / 'trap-mission-package-v1'
library = Path('level-editor/library')
manifest = json.loads((stage / 'manifest.json').read_text())
proof = root / 'full-editor-trap-variants-v1/verification.json'
verification = json.loads(proof.read_text())
assert verification['status'] == 'PASS' and len(verification['checks']) == 89
assert all(row['pass'] for row in verification['checks'])
assert verification['manifest_sha256'] == hashlib.sha256((stage / 'manifest.json').read_bytes()).hexdigest()
review = json.loads((proof.parent / 'root-review.json').read_text())
assert review['verification_sha256'] == hashlib.sha256(proof.read_bytes()).hexdigest()
self_review = json.loads((proof.parent / 'self-review.json').read_text())
for path, digest in self_review['runtime'].items():
    assert hashlib.sha256((Path('level-editor') / path).read_bytes()).hexdigest() == digest
for name, digest in self_review['evidence'].items():
    assert hashlib.sha256((proof.parent / name).read_bytes()).hexdigest() == digest
sha = lambda data: hashlib.sha256(data).hexdigest()
index = library / 'mission-states/index.json'
with (stage / 'install.lock').open('a') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    assert sha(index.read_bytes()) == manifest['previous_index_sha256']
    baseline = {str(p.relative_to(library)): sha(p.read_bytes())
                for p in (library / 'mission-states').rglob('*') if p.is_file()}
    for row in manifest['reused_files']:
        assert sha((library / row['path']).read_bytes()) == row['sha256']
    for row in manifest['new_files']:
        assert row['path'].startswith('mission-states/') and '..' not in Path(row['path']).parts
        data = (stage / 'library' / row['path']).read_bytes()
        assert sha(data) == row['sha256']
        destination = library / row['path']
        if destination.exists() and destination != index:
            assert destination.read_bytes() == data
    for row in manifest['new_files']:
        destination = library / row['path']
        if destination == index:
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        if not destination.exists():
            with destination.open('xb') as output:
                output.write((stage / 'library' / row['path']).read_bytes())
                output.flush()
                os.fsync(output.fileno())
    temporary = index.with_name('index.trap-variants-pending.json')
    with temporary.open('xb') as output:
        output.write((stage / 'library/mission-states/index.json').read_bytes())
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, index)
    for path, expected in baseline.items():
        if path != 'mission-states/index.json':
            assert sha((library / path).read_bytes()) == expected
    for row in manifest['new_files'] + manifest['reused_files']:
        assert sha((library / row['path']).read_bytes()) == row['sha256']
    receipt = {'status': 'INSTALLED', 'new_files': len(manifest['new_files']),
               'new_bytes': manifest['new_bytes'], 'reused_files': len(manifest['reused_files']),
               'baseline_files': len(baseline), 'baseline_nonindex_preserved': True,
               'index_sha256': sha(index.read_bytes()),
               'manifest_sha256': sha((stage / 'manifest.json').read_bytes()),
               'private_proof_sha256': sha(proof.read_bytes()),
               'scope': 'Native mission preview only; no scene or static asset publication.'}
    (stage / 'installation.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))
