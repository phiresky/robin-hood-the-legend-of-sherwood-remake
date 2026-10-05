"""Freeze exact ready cards with independently browsable, hash-bound resources."""
import argparse,json,shutil,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json


def freeze(destination):
    gallery=OUT/'gallery';digest=sha(gallery/'evidence.json')
    evidence=json.loads((gallery/'evidence.json').read_text())
    if destination.exists():raise FileExistsError(destination)
    destination.mkdir(parents=True)
    for name in ['evidence.json','index.html']:shutil.copy2(gallery/name,destination/name)
    files={}
    for row in evidence['items']:
        for kind in ['images','reports']:
            for value in row.get(kind,{}).values():
                source=Path(value['source']);target=destination/value['file']
                if sha(source)!=value['sha256']:raise ValueError('Gallery source changed: '+str(source))
                target.parent.mkdir(parents=True,exist_ok=True)
                if not target.exists():target.symlink_to(source)
                if sha(target)!=value['sha256']:raise ValueError('Snapshot resource changed: '+str(target))
                files[value['file']]=value['sha256']
    ready=[dict(id=r['id'],model=r['model'],model_sha256=sha(Path(r['model'])),review_revision=r['review_revision'])for r in evidence['items']if r['status']=='ready-for-user'and not r.get('user_approval')]
    if sha(gallery/'evidence.json')!=digest:raise ValueError('Gallery changed while freezing')
    write_json(destination/'manifest.json',dict(ready_count=len(ready),items=ready,evidence_sha256=digest,resources=files,scope='Exact ready geometry candidates only; no new user approval implied. Resources link to immutable hash-bound evidence, not mutable gallery paths.'))
    print(destination,len(ready),'ready',len(files),'resources')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('destination',type=Path);args=parser.parse_args();freeze(args.destination.resolve())
