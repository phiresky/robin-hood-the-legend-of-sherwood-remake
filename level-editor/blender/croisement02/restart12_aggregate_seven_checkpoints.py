"""Bind complete state-family checkpoints without treating counters as test results."""
from pathlib import Path
import hashlib,json,sys
ROOT=Path(__file__).resolve().parents[3];state=ROOT/'level-editor/work/croisement02-refinement/restart2-state'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
manifest_path=state/'remaining-seven-package-v2/manifest.json';manifest=read(manifest_path);expected={r['id']for r in manifest['entries']};checks={};families={};bindings=[];images=[];map_sha=None;runtime_sha=None
live_drift=[]
for name in ['installed-seven-http-checkpoint-a-v1','installed-seven-http-checkpoint-b-v1','installed-seven-http-northcart-v1','installed-seven-http-fence-emb05-v1','installed-seven-http-fence-tac02-v1','installed-seven-http-checkpoint-d-v1']:
 b=state/name;q=b/'browser/remaining-seven-states';salvage=name=='installed-seven-http-checkpoint-b-v1';receipt=b/'browser'/('post-termination-family-verification.json'if salvage else'verification.json');v=read(receipt)
 if salvage:
  assert v['status']=='PASS_COMPLETED_FAMILY_AFTER_TERMINATION';t=ROOT/v['completed_family']['path'];assert sha(t)==v['completed_family']['sha256'];family=read(t);whole={'checks':family['checks']};selected=[family['entry_id']];live_drift.append({'chunk':name,'files':v['runtime_drift']})
 else:
  assert v['status']=='PASS_SCOPED_STATE_CHUNK';assert read(b/'browser/process-final.json')['serverClosed'];live_drift.append({'chunk':name,**read(b/'browser/live-runtime-drift.json')});whole=read(q/'verification.json');assert whole['status']=='PASS'and whole['manifest_sha256']==sha(manifest_path);selected=v['selected_entry_ids']
 inp=read(b/'inputs.json');assert sha(b/'inputs.json')==v['inputs_sha256'];map_sha=map_sha or v['static_map_sha256'];assert map_sha==v['static_map_sha256'];runtime_sha=runtime_sha or sha(b/'runtime-baseline.json');assert runtime_sha==sha(b/'runtime-baseline.json')
 for c in whole['checks']:
  assert c['pass']is True
  if c['name']in checks:assert c['name']=='Map-only removes all state roots'
  checks[c['name']]=c
 for entry in selected:
  assert entry in expected and entry not in families;t=q/(entry+'-terminal.json');a=read(t);assert a['status']=='PASS'and a['entry_id']==entry and len(a['images'])==3;assert all(c['pass']is True for c in a['checks']);authority=next(e for e in manifest['entries']if e['id']==entry);assert a['contract_sha256']==authority['contract']['sha256'];assert a['checks']==[c for c in whole['checks']if c['name'].startswith(entry+' ')];assert {i['path']for i in a['images']}=={entry+'-'+mode+'.png'for mode in ['native','initial','applied']}
  for im in a['images']:
   p=q/im['path'];g=q/im['path'].replace('.png','-canvas-guard.json');assert sha(p)==im['sha256']and sha(g)==im['guard_sha256'];assert read(g)['status']=='PASS';images.append({'path':str(p.relative_to(ROOT)),'sha256':sha(p)})
  families[entry]={'path':str(t.relative_to(ROOT)),'sha256':sha(t),'checks':len(a['checks'])}
 bindings.append({'path':str(receipt.relative_to(ROOT)),'sha256':sha(receipt)})
assert set(families)==expected and len(checks)==97 and len(images)==21
contact=state/'installed-seven-http-contacts-v1/browser';v=read(contact/'verification.json');assert v['status']=='PASS_CONTACT_CAPTURE_PENDING_VISUAL_REVIEW';assert read(contact/'process-final.json')['serverClosed'];live_drift.append({'chunk':'contacts',**read(contact/'live-runtime-drift.json')});assert v['static_map_sha256']==map_sha
c=read(contact/'focused-state-contacts/verification.json');assert len(c['views'])==16
for row in c['views']:assert sha(contact/'focused-state-contacts'/row['image'])==row['sha256']
out=Path(sys.argv[1]).resolve();out.mkdir();result={'status':'PASS_NUMERIC_AND_CAPTURE_SCOPE_VISUAL_REVIEW_REQUIRED','manifest_sha256':sha(manifest_path),'static_map_sha256':map_sha,'runtime_baseline_sha256':runtime_sha,'checks':list(checks.values()),'live_runtime_drift':live_drift,'state_images':images,'family_terminals':families,'chunk_receipts':bindings,'contact_receipt':{'path':str((contact/'focused-state-contacts/verification.json').relative_to(ROOT)),'sha256':sha(contact/'focused-state-contacts/verification.json'),'views':16},'scope':'Exact97assertions21stateimages16contactimages. Snapshot evidence only; live metadata compatibility, contact visual review and publication gates remain separate.'};(out/'verification.json').write_text(json.dumps(result,indent=2)+'\n');print(out)
