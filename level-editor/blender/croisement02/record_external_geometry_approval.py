"""Archive a scoped external geometry card using an explicit user receipt."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json


def main(receipt_path):
    receipt=json.loads(receipt_path.read_text())
    if receipt['decision']!='approved' or receipt['scope']!='geometry only':raise ValueError('Explicit geometry-only user receipt required')
    evidence=Path(receipt['gallery_evidence']);assert sha(evidence)==receipt['gallery_evidence_sha256']
    item=next(r for r in json.loads(evidence.read_text())['items'] if r['id']==receipt['asset_id'] and r['review_revision']==receipt['review_revision'])
    model=Path(item['model']);assert sha(model)==receipt['model_sha256']
    binding={kind:{k:v['sha256'] for k,v in item[kind].items()} for kind in ['images','reports']};binding['model']=receipt['model_sha256']
    assert hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest()==receipt['review_revision']
    archive=OUT/'user-reviews'/item['id']/item['review_revision'];archive.mkdir(parents=True,exist_ok=True)
    for kind in ['images','reports']:
        for row in item[kind].values():
            relative=Path(row['file']);assert not relative.is_absolute() and '..' not in relative.parts
            source=evidence.parent/relative;target=archive/relative;assert sha(source)==row['sha256'];target.parent.mkdir(exist_ok=True)
            if not target.exists():shutil.copy2(source,target)
            assert sha(target)==row['sha256']
    if not (archive/'model.blend').exists():shutil.copy2(model,archive/'model.blend')
    assert sha(archive/'model.blend')==receipt['model_sha256']
    write_json(archive/'gallery-item.json',item);write_json(archive/'explicit-user-receipt.json',receipt)
    record=dict(asset_id=item['id'],decision='approved',scope='geometry',exact_user_text=receipt['user_text'],note='',review_revision=item['review_revision'],model_sha256=receipt['model_sha256'],archive=str(archive),solid_sha256=item['images']['solid']['sha256'],textured_sha256=item['images']['textured']['sha256'],gallery_evidence=str(evidence),gallery_evidence_sha256=sha(evidence),explicit_user_receipt=str(receipt_path),explicit_user_receipt_sha256=sha(receipt_path),scope_resolution='Exact external geometry card only; contextual state, shadows and textures are excluded',archive_scope='Exact model and displayed evidence; this external card does not claim a fresh standard workspace')
    write_json(archive/'decision.json',record)
    ledger=OUT/'user-feedback.json';data=json.loads(ledger.read_text());records={(r['asset_id'],r['review_revision']):r for r in data['records']};records[(item['id'],item['review_revision'])]=record
    write_json(ledger,dict(version=1,records=list(records.values())))
    print(archive)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('receipt',type=Path);main(parser.parse_args().receipt.resolve())
