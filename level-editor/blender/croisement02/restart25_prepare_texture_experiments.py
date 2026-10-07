"""Prepare reviewed approved-state texture experiments; never request generation."""
import hashlib,json,shlex,shutil,sys
import numpy as np
from PIL import Image
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE.parents[1]/'refinement')]
from prepare_texture_packet import prepare
ROOT=HERE.parents[2];BASE=ROOT/'level-editor/work/croisement02-refinement'
OUT=BASE/'restart25-approved-state-materialization-v1'
APPROVAL=BASE/'restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json'
EXPECTED='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert sha(APPROVAL)==EXPECTED
 user=json.loads(APPROVAL.read_text());members={m['asset_id']:m for c in user['decisions_by_card'] for m in c['members']}
 dest=OUT/'official-texture-experiments-v2';dest.mkdir(exist_ok=False)
 packets=[('hole-initial','hole-texture-inputs-v1/initial','croisement02-hole-initial'),('hole-applied','hole-texture-inputs-v1/applied','croisement02-hole-applied'),('mound-initial','mound-texture-inputs-v1/site-00','croisement02-hiding-mounds-all-initial-geometry')]
 results=[];commands=['#!/bin/sh','set -eu','cd '+shlex.quote(str(ROOT))]
 for key,relative,asset in packets:
  worker=OUT/relative;d=json.loads((worker/'derivation.json').read_text());a=members[asset]
  assert sha(d['source_model'])==a['model_sha256']==d['source_model_sha256'];assert sha(worker/'model.blend')==d['prepared_model_sha256']
  assert d['geometry_uv_materials_unchanged'] and all(v['protected_rgba_exact'] for v in d['views'])
  review_path=worker/'source-reference-review/root-review.json';review=json.loads(review_path.read_text());assert review['status']=='PASS_ROOT_INPUT_REVIEW'
  verify_path=worker/'source-reference-review/verification.json';assert sha(verify_path)==review['verification_sha256'];verification=json.loads(verify_path.read_text())
  assert len(verification['artifacts'])==review['verified_artifacts']
  for path,digest in verification['artifacts'].items():assert sha(worker/path)==digest,path
  target=dest/key;target.mkdir();workspace=target/'parent-workspace';workspace.mkdir();(workspace/'model.blend').symlink_to(d['source_model'])
  selection={'parent_geometry_revision':a['review_revision'],'preparation_model':str(worker/'model.blend')};selection_path=target/'preparation-selection.json';write(selection_path,selection)
  bridge={'kind':'approved-state-endpoint-preparation-derivative','source_user_decision':a,'exact_user_text':user['user_message'],'source_user_receipt_sha256':EXPECTED,'derivation':d,'root_input_review':review,'root_input_review_sha256':sha(review_path),'scope':'Exact displayed geometry scope and independently reviewed input derivative. Existing source projection stretch stays protected. Final generated appearance remains unapproved. Mound texture reuse is unknown surfaces only, with all20 observed domains restored.'};bridge_path=target/'approval-bridge.json';write(bridge_path,bridge)
  files=[APPROVAL,selection_path,bridge_path,*[p for p in worker.rglob('*') if p.is_file()]]
  evidence={str(p):{'path':str(p),'sha256':sha(p)} for p in files}
  identity={'asset_id':asset,'model_sha256':a['model_sha256'],'evidence':{k:v['sha256'] for k,v in evidence.items()}}
  revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
  item={'id':asset,'workspace':str(workspace),'status':'ready-for-user','stored_material_validation':'PASS','solid':str(worker/'modified/solid.png'),'textured':str(worker/'modified/textured.png'),'revision':{'sha256':revision,'model_sha256':a['model_sha256'],'evidence':evidence},'approval_provenance':bridge,'preparation_selection':str(selection_path),**selection}
  manifest=target/'review-manifest.json';decisions=target/'decisions.json';write(manifest,{'version':1,'items':[item]});write(decisions,{'version':1,'decisions':[{'asset_id':asset,'scope':'geometry','decision':'approved','exact_user_text':user['user_message'],'revision_sha256':revision,'original_gallery_decision':a,'translation':'Exact approved geometry scope with unchanged reviewed preparation; no final appearance approval inferred.'}]})
  exp=target/'experiment';prepare(manifest,asset,exp,decisions)
  refs=json.loads((worker/'source-reference-review/auxiliary-references.json').read_text());copied=[]
  for ref in refs['references']:
   source=Path(ref['file']);assert sha(source)==ref['sha256'];file=exp/('reference-'+source.name);shutil.copyfile(source,file);copied.append(dict(ref,file=str(file)))
  write(exp/'auxiliary-references.json',{'input_sha256':sha(exp/'input.png'),'lighting_sha256':sha(exp/'solid.png'),'references':copied})
  pixel_proof={}
  for name in ['input','solid','mask']:
   original=worker/'private-inputs'/f'{name}.png';prepared=exp/f'{name}.png';a=np.array(Image.open(original).convert('RGBA'));b=np.array(Image.open(prepared).convert('RGBA'));assert np.array_equal(a,b),name
   pixel_proof[name]={'reviewed_file_sha256':sha(original),'official_file_sha256':sha(prepared),'rgba_sha256':hashlib.sha256(a.tobytes()).hexdigest(),'rgba_exact':True}
  write(exp/'reviewed-pixel-equivalence.json',pixel_proof)
  command=['node','level-editor/pipeline/src/refinement/generate-textures.ts',str(exp),'--generate','--provider','openrouter','--prompt-variant','short','--no-mask','--lighting-reference',str(exp/'solid.png'),'--auxiliary-references',str(exp/'auxiliary-references.json'),'--prompt-suffix','Fill only the demonstrated unknown gray leaf-litter reverse and side surfaces. Preserve all known source detail, including existing projection stretch. Reference images provide material and color only; keep target geometry, camera layout and alpha unchanged.']
  commands.append(shlex.join(command));results.append({'key':key,'asset_id':asset,'experiment':str(exp),'input_sha256':sha(exp/'input.png'),'mask_sha256':sha(exp/'mask.png'),'solid_sha256':sha(exp/'solid.png'),'approval_sha256':sha(exp/'approval.json'),'views_sha256':sha(exp/'views.json'),'root_review_sha256':sha(review_path),'command':command})
 write(dest/'batch.json',{'status':'PREPARED_EXACT_REVIEWED_INPUTS_SANDBOX_DNS_UNAVAILABLE','dns_check':{'host':'openrouter.ai','result':'EAI_AGAIN temporary failure in name resolution','generation_attempted':False},'packets':results,'publication_allowed':False,'final_appearance_approved':False,'scope':'No API calls. Local editable masks remain active despite remote no-mask transport. Reuse mound unknown surface only, restore each of20 site observed domains.'})
 (dest/'generate-manually.sh').write_text('\n'.join(commands)+'\n');print(json.dumps({'batch':str(dest/'batch.json'),'sha256':sha(dest/'batch.json'),'experiments':len(results)}))
if __name__=='__main__':main()
