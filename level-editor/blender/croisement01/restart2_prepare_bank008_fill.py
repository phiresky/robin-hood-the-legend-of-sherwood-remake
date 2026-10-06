"""Translate approved joint geometry to its exact isolated soil texture receiver."""
import json,hashlib,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 asset='croisement01-tree01-bank008-joint';derivation=R/'bank008-approved-receiver-v2/derivation-proof.json';d=json.loads(derivation.read_text());w=derivation.parent/'assets'/asset
 card=R/'ready-tree01-wood-bank-geometry-metadata-v1.json';item=json.loads(card.read_text())['items'][0]
 approval=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/batch-v12/user-approval.json';decision=json.loads(approval.read_text());member=next(m for c in decision['cards'] for m in c['members'] if m['asset_id']=='croisement01-tree-01')
 assert decision['status']=='approved' and member['scope']=='geometry'
 assert member['model_sha256']==item['model_sha256']==d['parent_model_sha256']
 assert member['review_revision']==item['review_revision']
 assert d['receiver_model_sha256']==sha(w/'model.blend') and not d['gameplay_mutation']
 for name,h in item['evidence'].items():assert sha(Path(name))==h,name
 case=R/'approved-bank008-fill-v1'/asset;case.mkdir(parents=True,exist_ok=False)
 user=dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],batch_approval_path=str(approval),batch_approval_sha256=sha(approval),parent_asset_id=member['asset_id'],parent_model_sha256=member['model_sha256'],parent_review_revision=member['review_revision'],derivation_path=str(derivation),derivation_sha256=sha(derivation),model_sha256=sha(w/'model.blend'),scope_description='Exact approved bank008 joint world vertices/faces isolated for texture transport; no neighboring banks or tree materials included. Parent approval applies to this unchanged geometry.',texture_approval='pending');write(case/'user-decision.json',user)
 paths=[card,approval,derivation,case/'user-decision.json',w/'model.blend']+[p for p in (w/'modified').rglob('*') if p.is_file()]+[w/n for n in ['workspace.json','validation.json','handoff.json'] if (w/n).exists()]
 evidence={str(p):dict(path=str(p),sha256=sha(p)) for p in paths};identity=dict(asset_id=asset,model_sha256=sha(w/'model.blend'),evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 translated=dict(id=asset,workspace=str(w),status='ready-for-user',technical_eligible=True,generation_eligible=True,stored_material_validation='PASS',solid=str(w/'modified/solid.png'),textured=str(w/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=sha(w/'model.blend'),evidence=evidence),approval_provenance=user)
 write(case/'review-manifest.json',dict(version=1,items=[translated]));write(case/'decisions.json',dict(version=1,decisions=[dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],revision_sha256=revision,original_gallery_decision=user)]));prepare(case/'review-manifest.json',asset,case/'experiment',case/'decisions.json')
 e=case/'experiment';source=R/'tree01-soil-joint-v1/observed-soil-source.png'
 write(e/'auxiliary-references.json',dict(version=1,input_sha256=sha(e/'input.png'),lighting_sha256=sha(e/'solid.png'),references=[dict(source='material',file=str(source),sha256=sha(source),asset_id=asset,role='Own native soil only. Continue forest earth, leaf litter and moss palette on unknown bank surfaces; do not add trees or roots.',authorization='Own native artwork from the approved soil domain')]))
 print(case)
if __name__=='__main__':main()
