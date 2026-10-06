"""Prepare isolated approved Tree01 wood input; separate display-scope approval is pending."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from prepare_texture_packet import prepare
from restart2_texture_references import REFERENCES
from PIL import Image
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main(kind):
 assert kind=='01'
 cardname,batch,worker={'03':('ready-tree03-geometry-metadata-v1.json','batch-v13','tree03-v4'),'01':('ready-tree01-wood-bank-geometry-metadata-v1.json','batch-v12','tree01-soil-joint-v10')}[kind]
 asset='croisement01-tree-'+kind;w=R/worker/'assets'/asset;card=R/cardname;item=json.loads(card.read_text())['items'][0];approval=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches'/batch/'user-approval.json';decision=json.loads(approval.read_text());member=next(m for c in decision['cards'] for m in c['members'] if m['asset_id']==asset);assert member['scope']=='geometry' and member['model_sha256']==sha(w/'model.blend')==item['model_sha256'] and member['review_revision']==item['review_revision'];assert decision['status']=='approved'
 for name,h in item['evidence'].items():assert sha(Path(name))==h,name
 case=R/'approved-tree01-isolated-wood-fill-v1'/asset;case.mkdir(parents=True,exist_ok=(kind=='01'));assert not (case/'experiment').exists()
 user=dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],batch_approval_path=str(approval),batch_approval_sha256=sha(approval),model_sha256=member['model_sha256'],review_revision=member['review_revision'],scope_description=member['scope_description'],texture_approval='pending',input_scope_approval='pending grouped user decision before generation');write(case/'user-decision.json',user)
 packet=R/'tree01-isolated-wood-input-v1'
 paths=[card,approval,case/'user-decision.json',w/'model.blend']+[p for p in packet.rglob('*') if p.is_file()]+[w/n for n in ['workspace.json','validation.json','handoff.json'] if (w/n).exists()];evidence={str(p):dict(path=str(p),sha256=sha(p)) for p in paths};identity=dict(asset_id=asset,model_sha256=member['model_sha256'],evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 translated=dict(id=asset,workspace=str(w),status='ready-for-user',technical_eligible=True,generation_eligible=True,stored_material_validation='PASS',solid=str(packet/'solid.png'),textured=str(packet/'textured.png'),revision=dict(sha256=revision,model_sha256=member['model_sha256'],evidence=evidence),approval_provenance=user)
 write(case/'review-manifest.json',dict(version=1,items=[translated]));write(case/'decisions.json',dict(version=1,decisions=[dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],revision_sha256=revision,original_gallery_decision=user)]));prepare(case/'review-manifest.json',asset,case/'experiment',case/'decisions.json')
 e=case/'experiment';folder=e/'material-references';folder.mkdir();refs=[]
 for name,revision,box in REFERENCES:
  source=ROOT/'level-editor/work/leicester-refinement/round-1/texture-review/approved-evidence'/name/revision/'textured.png';target=folder/(name+'-leaf-material.png');Image.open(source).crop(box).save(target);refs.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=name,role='Permitted supplementary leaf material and palette only; preserve target geometry, cameras and all native pixels. Do not copy layout.',parent_image=str(source),parent_sha256=sha(source),crop_box=list(box),crop_method='Exact pixels, no resampling or recoloring',authorization='User explicitly permitted this Leicester tree'))
 write(e/'auxiliary-references.json',dict(version=1,input_sha256=sha(e/'input.png'),lighting_sha256=sha(e/'solid.png'),references=refs));print(case)
if __name__=='__main__':main(sys.argv[1])
