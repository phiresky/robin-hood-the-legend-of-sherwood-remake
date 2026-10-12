"""Reconcile current York evidence without treating historical ledger totals as fresh audit."""
import hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/active/york/restart2';OUT=B/'restart43-completion-inventory-v1';assert not OUT.exists();OUT.mkdir()
def pin(p):return dict(path=str(p),sha256=hashlib.sha256(p.read_bytes()).hexdigest())
source_files=[B/'full-scope-ledger.json',B/'live-work.json',B/'progress.json',ROOT/'level-editor/work/active/croisement02/continuation-20261009-root.json']
pair=B/'pair-textures-v1/assembled-v3-bounded/export-v2/publication-v1/publication.json';hall=B/'hall-textures-v2/exports-v3/independent-wiring-v2/publication-v1/publication.json';source_files.extend([pair,hall]);p=json.loads(pair.read_text());h=json.loads(hall.read_text());installed=[]
for path,digest in p['published_files'].items():
 if Path(path).name in ['model.glb','asset.json']:installed.append(dict(**pin(Path(path)),receipt_sha256=digest,matches=pin(Path(path))['sha256']==digest))
for row in h['payload']:
 if Path(row['target']).name in ['model.glb','asset.json']:installed.append(dict(**pin(Path(row['target'])),receipt_sha256=row['sha256'],matches=pin(Path(row['target']))['sha256']==row['sha256']))
assert all(row['matches']for row in installed)
exports=[]
for directory in ['restart42-approved-well-export-v2','restart42-approved-storehouse-export-v1','restart43-approved-gatehouse-export-v1','restart43-approved-shed-export-v1']:
 handoff=B/directory/'integration-handoff.json';v=json.loads(handoff.read_text())
 for path,digest in v['files'].items():assert pin(Path(path))['sha256']==digest,path
 exports.append(dict(asset_id=v['asset_id'],handoff=pin(handoff),browser_request=pin(handoff.with_name('browser-request.json')),status='Approved exact private export and surface guard complete; browser/transaction/canonical installation pending'))
source_files.extend([B/'approved-gate-staging-plan-v1/plan.json',B/'approved-shed-jamb-motion-integration-v1/integration.json',B/'restart40-approved-integration-v1/integration-plan.json',B/'winch-supported-motion-v1/state-plan/integration-plan-v1.json'])
receipts=list((ROOT/'level-editor/work/active/croisement02/restart3-review-batches').glob('*/user-approval*.json'))
receipts+=list(B.glob('*/user*approval*.json'))
chain_hash='ebe41b23e0b4f3695eace56ed8fdaf57b4f03eb193932a737b66cee18e2f2422'
matching=[pin(p)for p in receipts if chain_hash in p.read_text()]
report=dict(status='EVIDENCE_PINNED_PARTIAL_RECONCILIATION_NOT_FULL_MAP_AUDIT',sources=[pin(p)for p in source_files],installed_receipt_payloads=installed,verified_installed_groups=['york-market-southeast-tall-narrow-house','york-southwest-square-west-house','york-castle-great-hall'],installed_scope='Pair appearance and four independent hall appearances remain exact. Hall neighboring door/collision behavior and candle flame runtime are explicitly excluded by its receipt.',approved_private_exports=exports,compact_stage=pin(B/'restart43-approved-static-stage-v3/plan.json'),strict_loader=pin(B/'restart43-approved-static-stage-v3/strict-loader.json'),next_blockers=[
 dict(priority=1,scope='Approved four static assets/components',remaining='Finish queued actual-model browser checks, root guarded transaction, final live map/group/part/palette checks. No new appearance generation or user review needed for these exact approved files.'),
 dict(priority=2,scope='Portcullis patch000',remaining='Both endpoint appearances and 45-pose jamb-contact motion have receipts, but moving gate is not in the static gatehouse export. Export independent scenery/endpoints, bind native timing/door/audio/reset semantics, verify full gatehouse sweep/support with corrected channels, and validate production runtime before publication.',evidence=pin(B/'approved-shed-jamb-motion-integration-v1/integration.json')),
 dict(priority=3,scope='Winch patch004 and independent room patch005',remaining='Exactly24 components have approved frame44 geometry/appearance. They need scoped material transfer to a complete approved mechanism. Supported return-chain/motion model has private evidence but no matching approval found in scanned receipts; root review/user scoped decision and hidden hardware textures remain unresolved. Preserve separate room reveal and native transparent initial state.',model=pin(B/'winch-supported-motion-v1/model.blend'),matching_scanned_receipts=matching,evidence=pin(B/'winch-supported-motion-v1/state-plan/integration-plan-v1.json')),
 dict(priority=4,scope='Installed hall runtime exclusions',remaining='Native neighbor door/collision transitions and candle flame runtime remain outside installed appearance scope; need separate implementation/source parity/runtime evidence.'),
 dict(priority=5,scope='Remaining full York map',remaining='Fresh asset-by-asset geometry/texture/ownership/state inventory is still required. Historical ledger lists249 other named groups and terrain pending, but its approvals/status fields are stale. Do not call249 a freshly verified remaining count. Missing scenery and mission Fog/Night/reachability remain unaudited completion obligations;252/253 is not a certified final denominator.')],receipt_search=dict(paths=len(receipts),scope='Top-level grouped batch user-approval*.json and direct York worker user*approval*.json; not proof no other archive exists.'),no_current_percentage=True,reason='No up-to-date comprehensive York denominator or broad completion audit exists in the checked evidence.',canonical_writes=False)
(OUT/'inventory.json').write_text(json.dumps(report,indent=2)+'\n');print(pin(OUT/'inventory.json'))
