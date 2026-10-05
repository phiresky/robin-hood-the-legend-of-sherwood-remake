"""Archive only Croisement02 geometry members of an explicitly approved frozen batch."""
import argparse,json,shutil,sys
from pathlib import Path
from catalog import OUT
sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'level-editor/refinement/blender'))
from evidence_io import sha,write_json

def main(receipt_path, assets=None):
    receipt=json.loads(receipt_path.read_text())
    if receipt['decision']!='approved' or receipt['scope']!='all displayed cards' or not receipt['exact_user_text'].strip():
        raise ValueError('Explicit approval of the displayed batch required')
    evidence=Path(receipt['batch_evidence'])
    if sha(evidence)!=receipt['batch_evidence_sha256']:raise ValueError('Approved batch changed')
    data=json.loads(evidence.read_text())
    members=[m for c in data['cards'] for m in c['members'] if m['scope']=='geometry' and m['asset_id'].startswith('croisement02-')]
    if assets:
        wanted=set(assets)
        members=[m for m in members if m['asset_id'] in wanted]
        if {m['asset_id'] for m in members}!=wanted:raise ValueError('Requested asset absent from approved geometry scope')
    if not members:raise ValueError('No Croisement02 geometry decisions in approved batch')
    # Verify every scoped model and displayed resource before the first ledger write.
    for m in members:
        if sha(m['model'])!=m['model_sha256']:raise ValueError('Approved model changed: '+m['asset_id'])
        for row in m['images']+m['reports']:
            rel=Path(row['file'])
            if rel.is_absolute() or '..' in rel.parts:raise ValueError('Unsafe archived resource path')
            if sha(evidence.parent/rel)!=row['sha256']:raise ValueError('Displayed evidence changed')
    records=[]
    for m in members:
        archive=OUT/'user-reviews'/m['asset_id']/m['review_revision'];archive.mkdir(parents=True,exist_ok=True)
        for row in m['images']+m['reports']:
            target=archive/row['file'];target.parent.mkdir(parents=True,exist_ok=True)
            if not target.exists():shutil.copy2(evidence.parent/row['file'],target)
            if sha(target)!=row['sha256']:raise ValueError('Archived resource changed')
        if not (archive/'model.blend').exists():shutil.copy2(m['model'],archive/'model.blend')
        if sha(archive/'model.blend')!=m['model_sha256']:raise ValueError('Archived model changed')
        write_json(archive/'grouped-gallery-member.json',m)
        write_json(archive/'explicit-user-receipt.json',receipt)
        record=dict(asset_id=m['asset_id'],decision='approved',scope='geometry',exact_user_text=receipt['exact_user_text'],note='',review_revision=m['review_revision'],model_sha256=m['model_sha256'],archive=str(archive),gallery_evidence=str(evidence),gallery_evidence_sha256=sha(evidence),explicit_user_receipt=str(receipt_path),explicit_user_receipt_sha256=sha(receipt_path),scope_resolution='Exact Croisement02 geometry member from explicitly approved grouped batch; textures, other maps and later candidates excluded',archive_scope='Exact model and all displayed image/report evidence; frozen member retains original source revision and provenance')
        write_json(archive/'decision.json',record);records.append(record)
    ledger=OUT/'user-feedback.json';old=json.loads(ledger.read_text());indexed={(r['asset_id'],r['review_revision']):r for r in old['records']}
    for r in records:indexed[(r['asset_id'],r['review_revision'])]=r
    write_json(ledger,dict(version=1,records=list(indexed.values())))
    write_json(evidence.parent/'croisement02-geometry-approval-recorded.json',dict(explicit_user_receipt=str(receipt_path),explicit_user_receipt_sha256=sha(receipt_path),records=records))
    print('Archived exact geometry decisions:',len(records))

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('receipt',type=Path);parser.add_argument('--asset',action='append');args=parser.parse_args();main(args.receipt.resolve(),args.asset)
