"""Re-encode existing lossy models and previews without rebaking or simplifying.

Stage into a fresh --work directory; --apply installs under the publication lock,
with source checks and backups. Authoring models and textures are never modified.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'blender'))
from lossy_assets import (LibraryLock, PIPELINE, atomic_write, generate_asset_index,
                          sha, verify_derivatives, write_asset_index)


def recompress(root, work, apply=False):
    root, work = Path(root).resolve(strict=True), Path(work).resolve()
    if work.is_relative_to(root) or root.is_relative_to(work):
        raise ValueError('Work directory must be outside the library')
    work.mkdir(parents=True, exist_ok=False)
    index = generate_asset_index(root)
    problems = verify_derivatives(root, index=index)
    if problems:
        raise ValueError('Invalid derivatives: ' + '; '.join(problems))
    expected, jobs, receipts = {}, {}, {}

    def pin(name):
        expected.setdefault(name, sha(root / name))

    for entry in index['assets']:
        pin(entry['model'])
        pin(entry['descriptor'])
        for field in ('lossy_model', 'preview_model'):
            name = entry.get(field)
            if not name:
                continue
            pin(name)
            pin(name + '.receipt.json')
            receipt = json.loads((root / (name + '.receipt.json')).read_text())
            receipts[name] = receipt
            jobs[name] = {'input': str(root / name), 'output': str(work / 'staged' / name),
                          'preview': field == 'preview_model', 'sha256': expected[name]}
    manifest = work / 'jobs.json'
    manifest.write_text(json.dumps(list(jobs.values())))
    subprocess.run(['node', str(PIPELINE / 'src/meshopt-glb.ts'), '--batch', str(manifest)], check=True)
    files, rows = {}, []
    for name, job in jobs.items():
        staged = Path(job['output'])
        receipt = receipts[name]
        receipt['output'] = sha(staged)
        if job['preview']:
            source = receipt.get('source_model')
            if source in jobs:
                receipt['source'] = sha(Path(jobs[source]['output']))
            receipt['compression'] = 'meshopt-v1'
            # Keep its generation fingerprint: a codec migration does not certify old bake settings.
        else:
            receipt.setdefault('settings', {})['geometry_compression'] = 'meshopt-v1-if-smaller'
        staged_receipt = Path(str(staged) + '.receipt.json')
        staged_receipt.write_text(json.dumps(receipt, indent=2) + '\n')
        files[name] = staged
        files[name + '.receipt.json'] = staged_receipt
        rows.append({'path': name, 'before': (root / name).stat().st_size, 'after': staged.stat().st_size,
                     'preview': job['preview']})
    report = {'applied': False, 'files': rows,
              'before': sum(r['before'] for r in rows), 'after': sum(r['after'] for r in rows)}
    (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    if apply:
        with LibraryLock(root):
            for name, checksum in expected.items():
                if sha(root / name) != checksum:
                    raise ValueError(f'Asset changed while compressing: {name}; rerun with a fresh work directory')
            generate_asset_index(root, files=files)
            backup = work / 'backup'
            backup.mkdir()
            shutil.copyfile(root / 'index.json', backup / 'index.json')
            changed = [name for name, staged in files.items() if sha(staged) != expected[name]]
            for name in changed:
                saved = backup / name
                saved.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(root / name, saved)
            (work / 'restore.json').write_text(json.dumps({'root': str(root), 'files': changed + ['index.json']}, indent=2))
            try:
                for name in changed:
                    atomic_write(root / name, files[name].read_bytes())
                write_asset_index(root)
                problems = verify_derivatives(root)
                if problems:
                    raise ValueError('; '.join(problems))
            except BaseException:
                for name in changed + ['index.json']:
                    atomic_write(root / name, (backup / name).read_bytes())
                raise
            report['applied'] = True
            (work / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1] / 'library/3d-assets')
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    result = recompress(args.root, args.work, args.apply)
    print(json.dumps({k: v for k, v in result.items() if k != 'files'}, indent=2))
