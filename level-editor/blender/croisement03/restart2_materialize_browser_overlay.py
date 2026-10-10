"""Materialize a private, read-only HTTP library without copying large model blobs."""
import hashlib
import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
STAGE = ROOT / 'level-editor/work/croisement03-refinement/restart2/combined-trio-stage-v2'
SOURCE = STAGE / 'browser-library-overlay-v1'
OUT = STAGE / 'browser-library-overlay-v2'
FIXTURE = STAGE / 'browser-check-fixture-v2'


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    assert not OUT.exists() and not FIXTURE.exists(), 'Use a new output version'
    request = json.loads((SOURCE / 'browser-request.json').read_text())
    # Verify the exact staged inputs before allocating the new directory tree.
    for name, digest in request['pins'].items():
        assert sha(name) == digest, f'Stale source pin: {name}'
    OUT.mkdir()
    destination = OUT / 'library'
    source_library = SOURCE / 'library'
    files = []
    skipped = []

    def visit(source, target, ancestors=()):
        relative = source.relative_to(source_library).as_posix()
        if source.name.startswith('.') or source.name == 'backups':
            skipped.append(relative)
            return
        resolved = source.resolve(strict=True)
        if relative == 'game-data':
            target.symlink_to(resolved, target_is_directory=True)
            return
        if resolved.is_dir():
            assert resolved not in ancestors, f'Symlink directory cycle: {source}'
            target.mkdir()
            for child in sorted(source.iterdir()):
                visit(child, target / child.name, (*ancestors, resolved))
        else:
            assert resolved.is_file(), f'Unsupported library entry: {source}'
            os.link(resolved, target)
            a, b = resolved.stat(), target.stat()
            assert (a.st_dev, a.st_ino, a.st_size) == (b.st_dev, b.st_ino, b.st_size)
            assert not target.is_symlink()
            files.append({'path': relative, 'source': str(resolved), 'bytes': b.st_size, 'inode': b.st_ino})

    destination.mkdir()
    for source in sorted(source_library.iterdir()):
        visit(source, destination / source.name)
    pins = {}
    for name, digest in request['pins'].items():
        path = Path(name)
        try:
            target = destination / path.relative_to(source_library)
        except ValueError:
            target = path
        assert sha(target) == digest, f'Materialized pin differs: {target}'
        pins[str(target)] = digest
    request.update(library=str(destination), map=str(destination / 'scenes/croisement03.rhlos-map.json'),
                   pins=pins, catalog_overlay=str(OUT / 'overlay-receipt.json'),
                   status='READY materialized exact-byte private library; browser proof pending')
    request['limitations'] = [x for x in request['limitations'] if 'read-through symlink' not in x]
    request['limitations'].append('Hardlinked blobs are read-only inputs: never save, optimize or rewrite library files through this private view.')
    write(OUT / 'browser-request.json', request)
    receipt = {'status': 'PASS CPU materialization and all request pin checks', 'source': str(SOURCE),
               'library': str(destination), 'file_count': len(files), 'logical_bytes': sum(x['bytes'] for x in files),
               'files': files, 'skipped_http_disallowed': skipped, 'pins': pins,
               'allowed_top_level_symlink': 'game-data', 'canonical_written': False,
               'map_sha256': sha(request['map']), 'index_sha256': sha(destination / '3d-assets/index.json')}
    write(OUT / 'overlay-receipt.json', receipt)
    FIXTURE.mkdir()
    old_fixture = STAGE / 'browser-check-fixture-v1'
    config = json.loads((old_fixture / 'config.json').read_text())
    config.update(library=str(destination), map=request['map'])
    for name in ('clockModule', 'sceneryModule'):
        config[name] = config[name].replace('/@fs//', '/@fs/')
    write(FIXTURE / 'config.json', config)
    (FIXTURE / 'check.js').write_bytes((old_fixture / 'check.js').read_bytes())
    plan = json.loads((old_fixture / 'hook-plan.json').read_text())
    plan['browserRequest'] = str(OUT / 'browser-request.json')
    plan['pins'] = {str(FIXTURE / name): sha(FIXTURE / name) for name in ('check.js', 'config.json')}
    write(FIXTURE / 'hook-plan.json', plan)
    print(json.dumps({k: receipt[k] for k in ('status', 'library', 'file_count', 'logical_bytes', 'map_sha256', 'index_sha256')}))


if __name__ == '__main__':
    main()
