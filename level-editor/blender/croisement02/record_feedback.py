"""Archive pasted gallery decisions against their exact displayed revision."""
import argparse
import json
import re
import shutil
import sys
from pathlib import Path
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement/blender'))
from evidence_io import sha,write_json


def main():
    parser=argparse.ArgumentParser();parser.add_argument('feedback',type=Path);args=parser.parse_args()
    records=[]
    for line in args.feedback.read_text().splitlines():
        match=re.fullmatch(r'(croisement02-[\w-]+): (approved|needs refinement|feedback)(?: — (.*?))? \[review ([0-9a-f]{16})\]',line)
        if not match:continue
        asset,decision,note,prefix=match.groups();found=[]
        for directory in [OUT/'gallery',*sorted((OUT/'gallery/history').glob('*'))]:
            path=directory/'evidence.json'
            if not path.exists():continue
            for row in json.loads(path.read_text())['items']:
                if row['id']==asset and row['review_revision'].startswith(prefix):found.append((directory,row))
        if not found:raise ValueError('Reviewed revision not found: '+line)
        directory,item=found[0];model=Path(item['model']);model_hash=sha(model)
        binding={kind:{k:v['sha256'] for k,v in item[kind].items()} for kind in ('images','reports')};binding['model']=model_hash
        import hashlib
        if hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest()!=item['review_revision']:
            raise ValueError('Current model differs from reviewed revision: '+asset)
        archive=OUT/'user-reviews'/asset/item['review_revision'];archive.mkdir(parents=True,exist_ok=True)
        for kind in ('images','reports'):
            for value in item[kind].values():
                source=directory/value['file'];target=archive/value['file'];target.parent.mkdir(exist_ok=True)
                if sha(source)!=value['sha256']:raise ValueError('Gallery evidence changed')
                if not target.exists():shutil.copy2(source,target)
                if sha(target)!=value['sha256']:raise ValueError('Archived evidence changed')
        if not (archive/'model.blend').exists():shutil.copy2(model,archive/'model.blend')
        if sha(archive/'model.blend')!=model_hash:raise ValueError('Archived model changed')
        if decision=='approved':
            for name in ('modified','input','reference'):
                if not (archive/name).exists():shutil.copytree(model.parent/name,archive/name)
            for name in ('workspace.json','source-masks.json'):
                if (model.parent/name).exists() and not (archive/name).exists():shutil.copy2(model.parent/name,archive/name)
        write_json(archive/'gallery-item.json',item)
        record=dict(asset_id=asset,decision=decision,note=note or '',exact_user_text=line,scope='geometry',review_revision=item['review_revision'],model_sha256=model_hash,archive=str(archive),solid_sha256=item['images']['solid']['sha256'],textured_sha256=item['images']['textured']['sha256'])
        write_json(archive/'decision.json',record);records.append(record)
    if not records:raise ValueError('No gallery decisions in input')
    target=OUT/'user-feedback.json';old=json.loads(target.read_text()) if target.exists() else {'records':[]}
    by_revision={(r['asset_id'],r['review_revision']):r for r in old['records']}
    by_revision.update({(r['asset_id'],r['review_revision']):r for r in records})
    write_json(target,dict(version=1,records=list(by_revision.values())))
    print('Archived',len(records),'exact-revision reviews')

if __name__=='__main__':main()
