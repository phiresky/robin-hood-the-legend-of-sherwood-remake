"""Bind hub-approved Tree11 wood geometry to its guarded private texture input."""
import sys,json,hashlib,shutil
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from prepare_texture_packet import prepare
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';CASE=B/'approved-hub-textures-v1/croisement03-tree-11/wood-input-v1';ASSET='croisement03-tree-11'
def main():
 a=B/'approved-hub-v17-v23-plus-two-v1';archive=json.loads((a/'verified-scope.json').read_text());receipt=json.loads((a/'user-approval.json').read_text());assert receipt['status']=='USER_APPROVED' and sha(a/'user-approval.json')==archive['receipt_sha256'];approval=archive['effective_assets'][ASSET];assert approval['scope']=='geometry';norm=json.loads((CASE/'normalization.json').read_text());native=json.loads((CASE/'native-audit.json').read_text());assert native['accepted_bark_changes']==native['provisional_foliage_changes']==0;assert norm['approved_model_sha256']==approval['model_sha256'];workspace=CASE/'asset';validation=json.loads((workspace/'validation.json').read_text());assert validation['status']=='PASS';out=CASE/'packet-v1';out.mkdir(exist_ok=False);evidence={}
 def bind(key,p):evidence[key]=dict(path=str(p.resolve()),sha256=sha(p))
 for p in workspace.rglob('*'):
  if p.is_file() and p.suffix not in ('.blend','.blend1'):bind('workspace/'+str(p.relative_to(workspace)),p)
 bind('model',workspace/'model.blend')
 for p in [a/'verified-scope.json',a/'user-approval.json',CASE/'normalization.json',CASE/'normalized.blend',CASE/'native-audit.json',CASE/'native-comparison.png',CASE/'prepared.json']:bind('guard/'+str(p.relative_to(B)),p)
 frames=json.loads((workspace/'modified/views.json').read_text())
 for i,(p,h) in enumerate(sorted(frames['source_mask_evidence'].items())):assert sha(p)==h;bind('mask/'+str(i),Path(p))
 identity=dict(asset_id=ASSET,model_sha256=sha(workspace/'model.blend'),evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest();item=dict(id=ASSET,workspace=str(workspace),status='ready-for-user',technical_eligible=True,generation_eligible=True,stored_material_validation='PASS',solid=str(workspace/'modified/solid.png'),textured=str(workspace/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=identity['model_sha256'],evidence=evidence),approval_provenance=approval)
 decision=dict(asset_id=ASSET,scope='geometry',decision='approved',exact_user_text=receipt['user_message'],revision_sha256=revision,original_review_revision=approval['review_revision'],original_decision=str(a/'verified-scope.json'),translation='Approved Tree11 geometry contains these exact woody surfaces. Mechanical source-part join and ownership texture preparation only; all210 observed bark pixels and2805 crown pixels retain native first-hit identity in the normalized model. Original known shader/RGBA will be restored over final inferred fill; no new geometry or source assignment approved.')
 write_json(out/'review-manifest.json',dict(version=1,items=[item]));write_json(out/'decisions.json',dict(version=1,decisions=[decision]));result=prepare(out/'review-manifest.json',ASSET,out/'experiment',out/'decisions.json');write_json(out/'prepared.json',result)
 experiment=out/'experiment';refs=experiment/'material-references';refs.mkdir();rows=[]
 for name,box in [('leicester-southeast-cottage-tree',(359,520,403,589)),('leicester-moat-bank-tree',(369,471,390,532))]:
  source=B/'texture-batch-v7/croisement03-tree-25/experiment/material-references'/f'{name}.png';target=refs/f'{name}-bark.png';Image.open(source).crop(box).save(target);rows.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=name,role='Permitted observed bark texture character only. Do not copy object layout, foliage, black background or neutral unknown patches. Target slender gray-brown stems and branch geometry remain exact.',parent_image=str(source),parent_sha256=sha(source),crop=list(box)))
 write_json(experiment/'auxiliary-references.json',dict(version=1,input_sha256=sha(experiment/'input.png'),lighting_sha256=sha(experiment/'solid.png'),references=rows));print(experiment)
if __name__=='__main__':main()
