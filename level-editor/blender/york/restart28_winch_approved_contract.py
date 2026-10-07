"""Reconcile explicit geometry/motion approvals into the private state handoff."""
import copy,hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';OUT=WORK/'winch-integration-contract-v2/contract.json'
if OUT.exists():raise FileExistsError(OUT)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
receipt=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json';assert sha(receipt)=='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4';user=json.loads(receipt.read_text());ids={'york-castle-winch-stable-components','york-castle-winch--chain-and-hardware','york-castle-winch--shown-motion45'};members=[m for c in user['decisions_by_card']for m in c['members']if m['asset_id']in ids];assert{m['asset_id']for m in members}==ids
for m in members:
 assert sha(m['model'])==m['model_sha256']
 if 'source_evidence'in m:assert sha(m['source_evidence'])==m['source_evidence_sha256']
prior=WORK/'winch-integration-contract-v1/contract.json';contract=json.loads(prior.read_text())
for pin in contract['pins']:assert sha(pin['path'])==pin['sha256']
trace=copy.deepcopy(contract['trace']);contract['status']='PRIVATE_GEOMETRY_AND_SHOWN_MOTION_APPROVED_TEXTURE_AND_RUNTIME_PENDING';contract['review']={'stableGeometry':'User approved exact24 component scope','chainHardwareAndMotion':'User approved exact76links,9support components and shown45poses','texture':'Dependent stable24 packet prepared; provider blocked by sandbox DNS. Final appearance approval pending.','publication':'Root-only publication after guarded export and runtime validation'};contract['approval']={'receipt':str(receipt),'sha256':sha(receipt),'user_message':user['user_message'],'members':members};contract['previous_contract']={'path':str(prior),'sha256':sha(prior)};contract['live_edits']=False;assert contract['trace']==trace;OUT.parent.mkdir();OUT.write_text(json.dumps(contract,indent=2)+'\n');print(json.dumps({'output':str(OUT),'resources_verified':len(contract['pins']),'trace_rows':len(trace),'approved_scopes':len(members)}))
