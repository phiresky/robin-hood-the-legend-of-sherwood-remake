"""Bind explicit approved scopes and private scatter exports without publication."""
import copy, hashlib, json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
BASE=ROOT/'level-editor/work/croisement02-refinement'
DEST=BASE/'restart25-approved-state-materialization-v1'
EXPECTED='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def pin(p):return {'path':str(p.relative_to(ROOT)),'sha256':sha(p)}
def main():
 receipt=BASE/'restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json';assert sha(receipt)==EXPECTED
 approved=json.loads(receipt.read_text());members={m['asset_id']:m for c in approved['decisions_by_card'] for m in c['members']}
 prior=BASE/'restart24-hole-mound-bindings-v1/bindings.json';assert sha(prior)=='70a80e07119677bfcba5c50ca66dcd27423afaa2c339a87706d8de51a316d23b'
 d=json.loads(prior.read_text());old=copy.deepcopy(d)
 report_path=DEST/'scatter-export-v2/report.json';export=json.loads(report_path.read_text());assert export['status']=='PRIVATE_CPU_VERIFIED_BROWSER_PENDING'
 assert sha(export['model'])==export['model_sha256']
 for k in ['source_catalog','current_map','native_level','source_manifest']:
  p=d[k];assert sha(ROOT/p['path'])==p['sha256'],k
 ids={'hole-initial':'croisement02-hole-initial','hole-applied':'croisement02-hole-applied','mounds':'croisement02-hiding-mounds-all-initial-geometry','scatter':'croisement02-leaf-scatter-source-surfaces'}
 mapping={}
 for k,p in d['candidate_models'].items():
  member=members[ids[k]];assert p['sha256']==member['model_sha256']==sha(ROOT/p['path']);mapping[p['sha256']]=(k,member)
 source_approval=members['croisement02-remaining-native-patch-states']
 counts={'controls':0,'geometry_approved_endpoints':0,'appearance_approved_endpoints':0,'private_glb_endpoints':0,'absent_endpoints':0,'pending_texture_endpoints':0}
 for row in d['bindings']:
  assert sha(ROOT/row['source_contract']['path'])==row['source_contract']['sha256'];counts['controls']+=1
  row['source_approval']={'receipt':pin(receipt),'asset_id':source_approval['asset_id'],'scope':source_approval['scope'],'revision':source_approval['review_revision']}
  for phase,endpoint in row['endpoints'].items():
   if endpoint['kind']=='source-absent':counts['absent_endpoints']+=1;continue
   key,member=mapping[endpoint['model']['sha256']]
   endpoint['approval']={'receipt':pin(receipt),'asset_id':member['asset_id'],'scope':member['scope'],'revision':member['review_revision']}
   endpoint['geometry_user_approved']=True;counts['geometry_approved_endpoints']+=1
   endpoint['appearance_user_approved']=key=='scatter'
   endpoint['kind']='approved-worker-pending-materialization'
   if key=='scatter':
    matches=[s for s in export['scenes'] if row['id'] in s['instances']];assert len(matches)==1
    scene=matches[0];assert set(scene['objects'])==set(endpoint['selection']['names'])
    endpoint['private_runtime_candidate']={'path':export['model'],'sha256':export['model_sha256'],'model_scene':scene['name'],'placement':'saved-world-transform-no-additional-translation','verification':pin(report_path),'browser_appearance_verified':False,'receiver_context_verified':False}
    endpoint['kind']='approved-source-surface-private-glb';counts['appearance_approved_endpoints']+=1;counts['private_glb_endpoints']+=1
   else:
    endpoint['texture_completion_required']=True;counts['pending_texture_endpoints']+=1
   assert endpoint['runtime_asset'] is None
 assert counts=={'controls':63,'geometry_approved_endpoints':124,'appearance_approved_endpoints':32,'private_glb_endpoints':32,'absent_endpoints':2,'pending_texture_endpoints':92},counts
 # Approval bookkeeping and private assets must never silently alter native state behavior.
 for a,b in zip(old['bindings'],d['bindings']):
  for k in ['id','mission','profile','source_contract','terminal_tick','transition_duration','initially_active','definitive','physical_transition','startup_condition']:
   assert a[k]==b[k],(a['id'],k)
 d.update(schema='private-approved-patch-bindings.v1',status='APPROVED_GEOMETRY_PARTIAL_APPEARANCE_PRIVATE_STAGING',parent=pin(prior),approval_receipt=pin(receipt),materialization_counts=counts,
          approval_requirements=['Final hole and initial mound appearance approval after texture completion','Current receiver/aperture integration and browser verification','Root-owned guarded publication'],recipe=pin(Path(__file__).resolve()))
 d['publication_allowed']=False;d['production_contract']=False
 out=DEST/'approved-bindings.json';assert not out.exists();out.write_text(json.dumps(d,indent=2)+'\n');print(json.dumps({'path':str(out),'sha256':sha(out),'counts':counts}))
if __name__=='__main__':main()
