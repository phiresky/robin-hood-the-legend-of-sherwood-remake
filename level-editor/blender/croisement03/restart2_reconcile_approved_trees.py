"""Pin approved tree inputs and existing conversion artifacts before further work."""
import hashlib,json
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
RECEIPTS=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches'
OUT=B/'approved-trees02-11-reconciliation-v1'
def sha(p):
 with Path(p).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
def read(p):return json.loads(p.read_text())
def pin(p):return {'path':str(p),'sha256':sha(p)}
def main():
 assert not OUT.exists();OUT.mkdir()
 geometry=RECEIPTS/'pending-v17-v23-plus-two-hub-v1/user-approval.json';texture=RECEIPTS/'next-six-textures-channels-v1/user-approval.json'
 g=read(geometry);t=read(texture);assert g['status']=='USER_APPROVED' and t['user_text']=='all approved'
 members={m['asset_id']:m for c in g['decisions_by_card'] for m in c.get('members',[])}
 textures={m['asset_id']:m for m in t['decisions']}
 rows=[]
 for number in range(2,12):
  identity=f'croisement03-tree-{number:02}';m=members[identity];model=Path(m['model']);assert sha(model)==m['model_sha256']
  row={'asset_id':identity,'approved_geometry':pin(model),'geometry_scope':m['scope'],'geometry_receipt':pin(geometry)}
  if number==3:
   derivative=members[identity+'-scoped-leaf2787-derivative'];p=Path(derivative['model']);assert sha(p)==derivative['model_sha256'];row['effective_geometry']=pin(p);row['precedence']=g['tree03_approval_order']
  else:row['effective_geometry']=pin(model)
  if number in (10,11):
   entry=textures[identity+'-wood-texture'];p=Path(entry['model']);assert sha(p)==entry['model_sha256'];row.update(approved_appearance=pin(p),texture_receipt=pin(texture),texture_scope=entry['scope_description'])
   folder=B/f'approved-tree10-11-integration-v1/full-export-v1/tree{number}';report=read(folder/'report.json');assert report['source_sha256']==sha(p) and report['model_sha256']==sha(folder/'model.glb')
   row['existing_full_export']=pin(folder/'model.glb');row['content_guard']=report['content_guard'];row['native_proof']=pin(folder/'native-proof.json');row['metadata_proposal']=pin(B/f'approved-tree10-11-integration-v1/metadata-v1/{identity}/asset-metadata-proposal.json')
   row['next']='Derive inferred-only crown indices, retaining all wood/UV/texture bytes; exclude observed frame0 owner beside shared Arbre06. Then all-phase source/terrain and actual editor review. Full export is not installable alongside native animation.'
  else:row['next']='Prepare supplementary-reference bark fill inputs on exact approved geometry; confirm source ownership and native animation membership before export. No final unseen appearance approval inferred.'
  rows.append(row)
 request=B/'approved-tree10-11-integration-v1/browser-request.json';checked=read(request)['pins'];mismatch=[]
 for name,h in checked.items():
  p=Path(name)
  if not p.exists() or sha(p)!=h:mismatch.append(name)
 result={'status':'APPROVED_INPUTS_REHASHED_NOT_INSTALLED','rows':rows,'existing_export_request':pin(request),'existing_export_pins_checked':len(checked),'existing_export_pin_mismatches':mismatch,'canonical_written':False,'limits':['Geometry approval and bark approval do not establish final shared animation/terrain ownership.','No native animation or broad source domain is claimed from material role tags alone.']}
 (OUT/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'report':str(OUT/'report.json'),'sha256':sha(OUT/'report.json'),'trees':len(rows),'export_pin_mismatches':mismatch}))
if __name__=='__main__':main()
