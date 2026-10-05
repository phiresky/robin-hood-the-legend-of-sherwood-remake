"""Add original standard review packets to already approved grouped archives."""
import argparse,hashlib,json,shutil,sys
from pathlib import Path
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json

def main(assets):
    ledger=OUT/'user-feedback.json';data=json.loads(ledger.read_text());updated=[]
    for asset in assets:
        record=next(r for r in reversed(data['records']) if r['asset_id']==asset and r['decision']=='approved' and r['scope']=='geometry')
        archive=Path(record['archive']);member=json.loads((archive/'grouped-gallery-member.json').read_text());source=Path(member['source_evidence'])
        if sha(source)!=member['source_evidence_sha256'] or sha(archive/'model.blend')!=record['model_sha256']:raise ValueError('Approved source/model changed')
        item=next(i for i in json.loads(source.read_text())['items'] if i.get('id')==asset and i['review_revision']==record['review_revision'])
        binding={k:{n:r['sha256'] for n,r in item[k].items()} for k in ['images','reports']};binding['model']=record['model_sha256']
        if hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest()!=record['review_revision']:raise ValueError('Standard source revision changed')
        worker=Path(item['model']).parent
        for folder in ['input','modified','reference']:
            if not(worker/folder).is_dir():raise ValueError('Not a complete standard workspace: '+asset)
        for kind in ['images','reports']:
            for row in item[kind].values():
                src=source.parent/row['file'];target=archive/row['file'];target.parent.mkdir(parents=True,exist_ok=True)
                if sha(src)!=row['sha256']:raise ValueError('Frozen source resource changed')
                if not target.exists():shutil.copy2(src,target)
                if sha(target)!=row['sha256']:raise ValueError('Archive resource mismatch')
        for folder in ['input','modified','reference']:
            if not(archive/folder).exists():shutil.copytree(worker/folder,archive/folder)
            for src in (worker/folder).rglob('*'):
                if src.is_file() and sha(src)!=sha(archive/src.relative_to(worker)):raise ValueError('Archived workspace mismatch')
        for name in ['workspace.json','source-masks.json']:
            target=archive/name
            if not target.exists():shutil.copy2(worker/name,target)
            if sha(target)!=sha(worker/name):raise ValueError('Archived metadata mismatch')
        write_json(archive/'gallery-item.json',item)
        record['solid_sha256']=item['images']['solid']['sha256'];record['textured_sha256']=item['images']['textured']['sha256']
        record['standard_archive_extension']='Original source gallery revision and standard input/modified/reference packet preserved; no model, approval scope or review revision changed'
        write_json(archive/'decision.json',record);updated.append(asset)
    write_json(ledger,data);print('Extended existing approved archives:',updated)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('assets',nargs='+');main(parser.parse_args().assets)
