"""Bind newly approved York scopes to private export and state integration inputs."""
import hashlib,json,struct
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
W=ROOT/'level-editor/work/york-refinement/restart2'
OUT=W/'approved-shed-jamb-motion-integration-v1'
assert not OUT.exists()
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
receipt=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/next-climbing-shed-v2/user-approval.json'
assert sha(receipt)=='830263fe013c8a0157c158290e529e91188bf8bad6f984de090c53569795d088'
a=json.loads(receipt.read_text());assert a['status']=='approved' and a['exact_user_text']=='All four approved'
verified={}
def pin(p,expected=None):
 p=Path(p);digest=sha(p)
 if expected is not None:assert digest==expected,str(p)
 verified[str(p)]=digest
 return {'file':str(p),'sha256':digest}
pin(a['gallery_evidence'],a['gallery_evidence_sha256'])
for p,digest in a['verified_files'].items():pin(p,digest)
members=[x for x in a['members']if x['asset_id'].startswith('york-')]
assert len(members)==3
for x in members:
 assert x['decision']=='approved';pin(x['model'],x['model_sha256']);pin(x['source_evidence'],x['source_evidence_sha256'])
packet=W/'jamb-clearance-candidate-v1';freeze=json.loads((packet/'gallery-freeze.json').read_text())
for p,digest in freeze['files'].items():pin(packet/p,digest)
old=json.loads((W/'approved-gate-staging-plan-v1/plan.json').read_text())
# Keep earlier approved appearances and a fresh immutable baseline, not stale live hashes.
for m in old['approved_members']:pin(m['model'],m['model_sha256']);pin(m['source_evidence'],m['source_evidence_sha256'])
live={}
for asset in ['york-castle-west-gatehouse','york-riverside-storehouse-timber-shed']:
 d=ROOT/'level-editor/library/3d-assets/york'/asset
 for f in ['asset.json','model.glb']:live[str(d/f)]=sha(d/f)
for p in old['protected_live_inputs']:live[p]=sha(p)
shed=json.loads((ROOT/'level-editor/library/3d-assets/york/york-riverside-storehouse-timber-shed/asset.json').read_text())
gatehouse=json.loads((ROOT/'level-editor/library/3d-assets/york/york-castle-west-gatehouse/asset.json').read_text())
source=ROOT/'level-editor/work/york-refinement/geometry-pass-01/native-state-source-v1/manifest.json'
manifest=json.loads(source.read_text());native=next(x for x in manifest['records']if x['id']=='patch-000');r=native['record']
assert r['definitive'] and r['integrate_in_background'] and not r['end_animation_valid'] and r['door_indices']==[13,14]
motion_path=packet/'motion-proposal.json';pin(motion_path,'07fd102ae0a508bf77ff10f6d52dda6c746723d3da7011b312b9164b7758a446')
curve=json.loads(motion_path.read_text());assert len(curve['rows'])==45
transition=next(x for x in native['rows']if x['action']=='PatchTransition');assert len(transition['frames'])==45
native_frames=[]
for i,f in enumerate(transition['frames']):
 assert f['delay']==1
 native_frames.append({'frame':i,'source':pin(source.parent/f['image'],f['sha256']),'bbox':f['bbox'],'sound_id':f['sound_id'],'world_lift_z':curve['rows'][i]['nominal_lift_world_z']})
report={'status':'APPROVED_SCOPES_PRIVATE_INTEGRATION_PREPARATION','receipt':pin(receipt),'members':members,'verified_resource_count':len(verified),'verified_resources':verified,'protected_live_inputs':live,
 'export_operations':[
 {'asset':'york-riverside-storehouse-timber-shed','model':members[0]['model'],'scope':'Replace only asset-owned building-005 meshes with exact approved31receiver worker; preserve canonical source pivot, descriptor gameplay, interior, door and other assets.','source_origin_scene':shed['source_origin_scene'],'existing_parts':shed['parts'],'runtime_approval':False},
 {'asset':'york-castle-west-gatehouse','model':str(packet/'model.blend'),'scope':'Add only building-778 component portcullis-jamb-return; preserve complete existing778 and every other gatehouse part.','source_origin_scene':gatehouse['source_origin_scene'],'appearance_approval':'Prior b26c3658 jamb appearance carried through exact unchanged UV loops/images, validated by saved source sampling. No synthesis needed.','requires_before_export':'Read-only full saved gatehouse component merge and topology/material conservation check.'},
 {'asset':'york-castle-portcullis','model':old['approved_members'][0]['model'],'scope':'Export scenery-york-castle-portcullis only as independent authored scenery; keep separately approved covered/raised endpoint appearances.','physical_motion':pin(motion_path),'motion_approval_scope':'Exact45poses and contact to corrected jamb only. Frame36inferred57pixels within57–65. No complete gatehouse support/runtime approval.'}],
 'state_binding':{'patch':'patch-000','definitive':True,'initial':{'visible':True,'pose':0,'appearance':'covered'},'transition':'Sample authoritative frame0..44 after update; do not run a modulo clock. Reset counter65535 is valid only at frame0.','applied':{'visible':True,'pose':44,'appearance':'raised','persists_after_fx_inactive':True},'frame_period_updates':2,'completion_update':90,'native_frames':native_frames,'audio':'Native presentation owns sound361 on frame0; physical adapter emits none.','door_indices':[13,14],'independent_from':['patch-004 winch','patch-005 room cover'],'native_art':pin(source)},
 'separate_remaining_audits':['Whole gatehouse swept-volume/support contact; this approval does not certify it.','Source-node/component mesh preservation during private export.','Physical state adapter integration into authoritative patch snapshots and native playback parity.','Private roundtrip/export/browser validation when assigned lanes become available.','Root-only canonical publication after freeze release.'],
 'live_mutations':False,'exports_created':False}
OUT.mkdir();(OUT/'integration.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'out':str(OUT),'approved_york_scopes':len(members),'pinned_resources':len(verified),'gate_frames':len(native_frames)}))
