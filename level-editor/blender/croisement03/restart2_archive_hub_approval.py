"""Rehash scoped user approvals and preserve Tree03 derivative precedence."""
import hashlib,json,shutil
from pathlib import Path
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'approved-hub-v17-v23-plus-two-v1';RECEIPT=R/'level-editor/work/croisement02-refinement/restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json';EXPECTED='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
def sha(p):
 h=hashlib.sha256()
 with Path(p).open('rb') as f:
  for chunk in iter(lambda:f.read(1024*1024),b''):h.update(chunk)
 return h.hexdigest()
def main():
 assert sha(RECEIPT)==EXPECTED;d=json.loads(RECEIPT.read_text());assert d['status']=='USER_APPROVED';O.mkdir(exist_ok=False);cache={};members=[];freezes=[]
 def verify(p,h):
  p=Path(p).resolve();actual=cache.setdefault(str(p),sha(p));assert actual==h,(str(p),actual,h)
 for c in d['decisions_by_card']:
  for m in c['members']:
   if not m['asset_id'].startswith('croisement03-'):continue
   verify(m['model'],m['model_sha256']);row=dict(m,card_id=c['card_id'])
   if m.get('source_evidence'):
    verify(m['source_evidence'],m['source_evidence_sha256']);ep=Path(m['source_evidence']);freeze=ep.parent.parent/'freeze.json' if ep.parent.name=='gallery' else ep.parent/'freeze.json'
   else:freeze=B/'geometry-round15-tree02-shared-ridge-v1/freeze.json'
   if freeze.exists() and str(freeze) not in freezes:
    f=json.loads(freeze.read_text())
    for p,h in f['files'].items():verify(p,h)
    freezes.append(str(freeze))
   if m['scope']=='texture-input':
    for p,h in json.loads(Path(m['source_evidence']).read_text())['items'][0]['evidence'].items():verify(p,h)
   members.append(row)
 original=next(m for m in members if m['asset_id']=='croisement03-tree-03');derivative=next(m for m in members if m['asset_id']=='croisement03-tree-03-scoped-leaf2787-derivative');assert original['model_sha256']==d['tree03_approval_order']['original_model_sha256'];assert derivative['model_sha256']==d['tree03_approval_order']['derivative_model_sha256']
 effective={m['asset_id']:m for m in members if 'derivative' not in m['asset_id']};effective['croisement03-tree-03']={**derivative,'asset_id':'croisement03-tree-03','original_approved_baseline':original,'applied_after_original':True}
 shutil.copyfile(RECEIPT,O/'user-approval.json');report=dict(status='PASS scoped approval inputs rehashed',receipt_sha256=EXPECTED,exact_user_text=d['user_message'],members=members,effective_assets=effective,verified_files=cache,freeze_manifests=freezes,limits=['Exact displayed scopes only. Generated appearance remains pending.','Tree03 original approved first, then leaf2787 derivative.','Wholebank remains HOLD; no canonical writes.']);(O/'verified-scope.json').write_text(json.dumps(report,indent=2)+'\n');print({'members':len(members),'effective_assets':len(effective),'files':len(cache)})
if __name__=='__main__':main()
