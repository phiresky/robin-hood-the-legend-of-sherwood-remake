"""Share identical immutable gallery-history files without removing any paths."""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def identity(path):
    s = path.lstat()
    if not stat.S_ISREG(s.st_mode):
        raise ValueError(f'Not a regular file: {path}')
    return (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns,
            stat.S_IMODE(s.st_mode), s.st_uid, s.st_gid)


def run(root, receipt, apply=False):
    root = root.resolve(strict=True)
    if root.name != 'history' or root.parent.name != 'gallery':
        raise ValueError('Expected an immutable gallery/history directory')
    if receipt.exists() or receipt.resolve().is_relative_to(root):
        raise ValueError('Use a new receipt outside history')
    groups = collections.defaultdict(list)
    for directory, dirs, files in os.walk(root):
        if any((Path(directory) / name).is_symlink() for name in dirs):
            raise ValueError('History contains a directory symlink')
        for name in sorted(files):
            path = Path(directory) / name
            ident = identity(path)
            groups[(ident[2], *ident[4:])].append(path)
    count = sum(map(len, groups.values()))
    changed = recovered = 0
    print(json.dumps(dict(status='scanned', files=count)), flush=True)
    with receipt.open('x') as journal:
        for paths in groups.values():
            canonicals = {}
            for path in paths:
                before = identity(path)
                sha = digest(path)
                if identity(path) != before:
                    raise RuntimeError(f'File changed while hashing: {path}')
                canonical = canonicals.get(sha)
                if canonical is None:
                    canonicals[sha] = (path, before)
                    action = 'retained'
                else:
                    source, source_identity = canonical
                    if identity(source) != source_identity:
                        raise RuntimeError(f'Canonical changed: {source}')
                    action = 'already-shared'
                    if before[:2] != source_identity[:2]:
                        action = 'linked' if apply else 'would-link'
                        old = path.stat()
                        if apply:
                            fd, name = tempfile.mkstemp(prefix='.dedup-', dir=path.parent)
                            os.close(fd)
                            temporary = Path(name)
                            try:
                                temporary.unlink()
                                os.link(source, temporary)
                                if identity(path) != before:
                                    raise RuntimeError(f'Target changed: {path}')
                                os.replace(temporary, path)
                            finally:
                                temporary.unlink(missing_ok=True)
                        changed += 1
                        if old.st_nlink == 1:
                            recovered += old.st_blocks * 512
                journal.write(json.dumps(dict(path=str(path.relative_to(root)),
                    sha256=sha, action=action)) + '\n')
                if changed and changed % 10000 == 0 and action in ('linked', 'would-link'):
                    journal.flush()
                    print(json.dumps(dict(processed_duplicates=changed,
                                          reclaimed_bytes=recovered)), flush=True)
        journal.flush()
        os.fsync(journal.fileno())
    # Recheck every path; hash each final inode once to avoid rereading shared bytes.
    checked = {}
    with receipt.open() as journal:
        rows = 0
        for line in journal:
            row = json.loads(line)
            path = root / row['path']
            ident = identity(path)
            if ident not in checked:
                checked[ident] = digest(path)
            if checked[ident] != row['sha256']:
                raise RuntimeError(f'Final content mismatch: {path}')
            rows += 1
    if rows != count:
        raise RuntimeError('Final path count mismatch')
    result = dict(status='VERIFIED', apply=apply, files=count,
                  replacements=changed, reclaimed_bytes=recovered,
                  final_unique_inodes=len(checked))
    receipt.with_suffix('.summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('history', type=Path)
    parser.add_argument('receipt', type=Path)
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    run(args.history, args.receipt, args.apply)
