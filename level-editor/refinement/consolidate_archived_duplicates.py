"""Consolidate immutable archive contents while preserving every consumer path.

Only audited regular files under needs-review are eligible. Shared files become
read-only. Use `edit FILE` to detach an independent writable copy before editing.
No symlinks, path redirections, metadata rewrites, or mutable aliases are created.
"""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile


def identity(path):
    s = path.lstat()
    if not stat.S_ISREG(s.st_mode):
        raise ValueError('Expected a regular file: ' + str(path))
    return s


def digest(path):
    before = identity(path)
    with path.open('rb') as stream:
        value = hashlib.file_digest(stream, 'sha256').hexdigest()
    after = identity(path)
    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
        raise ValueError('File changed while hashing: ' + str(path))
    return value


def contained(root, name):
    p = root / name
    if not p.is_relative_to(root/'work/needs-review'):
        raise ValueError('Outside reviewed archive: ' + str(p))
    if p.resolve(strict=True) != p.absolute():
        raise ValueError('Archive path contains symlinks or traversal: ' + str(p))
    return p


def plan(root, audits):
    root = Path(root).resolve(); selected = []; skipped = []
    for audit in audits:
        for group in json.loads(Path(audit).read_text())['groups']:
            files = group['files']; paths = []; reason = None
            for row in files:
                try:
                    p = contained(root, row['path']); s = identity(p)
                    if (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns) != (row['dev'], row['ino'], row['bytes'], row['mtime_ns']):
                        reason = 'changed since audit'; break
                    paths.append((p, s, row))
                except (FileNotFoundError, ValueError) as e:
                    reason = str(e); break
            counts = collections.Counter((s.st_dev, s.st_ino) for p,s,row in paths)
            if not reason and any(s.st_nlink != counts[(s.st_dev,s.st_ino)] for p,s,row in paths):
                reason = 'additional hardlinks outside this audited group'
            if reason:
                skipped.append({'sha256':group['sha256'],'reason':reason,'files':[r['path'] for r in files]});continue
            # Keep the original main/approved source where available; all paths survive.
            paths.sort(key=lambda item:('/main/' not in str(item[0]),item[0].name != 'approved-model.blend',len(str(item[0])),str(item[0])))
            unique={(s.st_dev,s.st_ino):s.st_blocks*512 for p,s,row in paths}
            if len(unique)<2:continue
            keeper=paths[0]
            selected.append(dict(sha256=group['sha256'],keeper=keeper[2]['path'],files=[dict(row,original_mode=stat.S_IMODE(s.st_mode)) for p,s,row in paths],reclaimable_bytes=sum(unique.values())-unique[(keeper[1].st_dev,keeper[1].st_ino)]))
    return dict(version=1,status='PLANNED_NO_CHANGES',root=str(root),groups=selected,skipped=skipped,reclaimable_bytes=sum(g['reclaimable_bytes'] for g in selected),consumer_policy='Original paths and bytes preserved; shared archive contents are read-only. edit detaches an independent writable copy.')


def apply(plan_file, receipt_file):
    raw=Path(plan_file).read_bytes(); data=json.loads(raw); root=Path(data['root'])
    if data.get('version')!=1:raise ValueError('Unsupported plan')
    receipt_file=Path(receipt_file)
    if receipt_file.exists():raise FileExistsError(receipt_file)
    recovered=0; replacements=0;verified=0
    with receipt_file.open('x') as journal:
        journal.write(json.dumps(dict(event='begin',plan_sha256=hashlib.sha256(raw).hexdigest()))+'\n');journal.flush();os.fsync(journal.fileno())
        for n,group in enumerate(data['groups']):
            paths=[(contained(root,row['path']),row) for row in group['files']]
            current={};hashes={}
            for p,row in paths:
                s=identity(p)
                if (s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns)!=(row['dev'],row['ino'],row['bytes'],row['mtime_ns']):
                    raise ValueError('Archive changed since plan: '+str(p))
                key=(s.st_dev,s.st_ino)
                if key not in hashes:hashes[key]=digest(p)
                if hashes[key]!=group['sha256']:raise ValueError('Content differs from audited hash: '+str(p))
                current[p]=s
            counts=collections.Counter((s.st_dev,s.st_ino) for s in current.values())
            if any(s.st_nlink!=counts[(s.st_dev,s.st_ino)] for s in current.values()):
                raise ValueError('New external hardlink: '+group['keeper'])
            keeper=contained(root,group['keeper']);keeper_stat=current[keeper]
            journal.write(json.dumps(dict(event='group-prepared',group=n,sha256=group['sha256'],files=group['files']))+'\n');journal.flush();os.fsync(journal.fileno())
            # Freeze all old inodes before sharing; no in-place archive writer can
            # mutate another consumer's retained bytes after consolidation.
            for p,s in current.items():os.chmod(p,stat.S_IMODE(s.st_mode)&~0o222)
            for p,row in paths:
                s=identity(p)
                if (s.st_dev,s.st_ino)==(keeper_stat.st_dev,keeper_stat.st_ino):continue
                if (s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns)!=(row['dev'],row['ino'],row['bytes'],row['mtime_ns']):raise ValueError('Target changed before replacement: '+str(p))
                fd,name=tempfile.mkstemp(prefix='.archive-dedup-',dir=p.parent);os.close(fd);temporary=Path(name)
                try:
                    temporary.unlink();os.link(keeper,temporary);os.replace(temporary,p)
                finally:temporary.unlink(missing_ok=True)
                saved=s.st_blocks*512 if s.st_nlink==1 else 0
                recovered+=saved;replacements+=1
                journal.write(json.dumps(dict(event='consolidated',path=row['path'],keeper=group['keeper'],sha256=group['sha256'],reclaimed_bytes=saved))+'\n');journal.flush()
            if digest(keeper)!=group['sha256']:raise ValueError('Retained content changed')
            for p,row in paths:
                s=identity(p)
                if (s.st_dev,s.st_ino)!=(keeper_stat.st_dev,keeper_stat.st_ino) or s.st_mode&0o222:raise ValueError('Sharing or read-only verification failed: '+str(p))
                verified+=1
            journal.write(json.dumps(dict(event='group-verified',group=n,paths=len(paths)))+'\n');journal.flush();os.fsync(journal.fileno())
            if n%25==0:print(json.dumps(dict(groups_done=n+1,replacements=replacements,reclaimed_gib=round(recovered/2**30,3))),flush=True)
    result=dict(status='VERIFIED',groups=len(data['groups']),verified_paths=verified,replacements=replacements,reclaimed_bytes=recovered,deleted_paths=0,policy=data['consumer_policy'])
    receipt_file.with_suffix('.summary.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
    return result


def edit(path):
    """Detach before making writable, including a last remaining archive link."""
    path=Path(path).absolute();before=identity(path)
    if path.resolve(strict=True)!=path:raise ValueError('Refuse symlink edit path')
    expected=digest(path)
    fd,name=tempfile.mkstemp(prefix='.archive-edit-',dir=path.parent);os.close(fd);temporary=Path(name)
    try:
        shutil.copyfile(path,temporary)
        if digest(temporary)!=expected:raise ValueError('Independent copy differs')
        current=identity(path)
        if (current.st_dev,current.st_ino,current.st_size,current.st_mtime_ns,current.st_ctime_ns)!=(before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns):raise ValueError('Archive changed during detachment')
        os.chmod(temporary,stat.S_IMODE(before.st_mode)|stat.S_IWUSR)
        os.replace(temporary,path)
    finally:temporary.unlink(missing_ok=True)
    return dict(path=str(path),status='INDEPENDENT_WRITABLE_COPY',sha256=expected)


def main():
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest='command',required=True)
    p=sub.add_parser('plan');p.add_argument('--root',type=Path,default=Path.cwd());p.add_argument('--output',type=Path,required=True);p.add_argument('audits',nargs='+',type=Path)
    p=sub.add_parser('apply');p.add_argument('plan',type=Path);p.add_argument('receipt',type=Path)
    p=sub.add_parser('edit');p.add_argument('path',type=Path)
    args=parser.parse_args()
    if args.command=='plan':
        result=plan(args.root,args.audits)
        with args.output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
        print(json.dumps(dict(groups=len(result['groups']),skipped=len(result['skipped']),reclaimable_bytes=result['reclaimable_bytes'])))
    elif args.command=='apply':apply(args.plan,args.receipt)
    else:print(json.dumps(edit(args.path)))

if __name__=='__main__':main()
