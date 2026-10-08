"""Bind local climbing texture packets to explicit geometry approvals; no API calls."""
import json,hashlib,shlex,shutil,sys
from pathlib import Path
import numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[2]
sys.path.insert(0,str(HERE.parents[1]/'refinement'))
from prepare_texture_packet import prepare
BASE=ROOT/'level-editor/work/croisement02-refinement';SOURCE=BASE/'restart14-hidden-archer/climbing-v21-edge';INPUT=BASE/'restart14-hidden-archer/climbing-texture-inputs-v1';DEST=BASE/'restart14-hidden-archer/climbing-texture-experiments-v1'
APPROVAL=BASE/'restart3-review-batches/next-climbing-shed-v2/user-approval.json'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 assert sha(APPROVAL)=='830263fe013c8a0157c158290e529e91188bf8bad6f984de090c53569795d088';assert not DEST.exists();DEST.mkdir()
 user=json.loads(APPROVAL.read_text());members={m['asset_id']:m for m in user['members']};batch=[];commands=['#!/bin/sh','set -eu','cd '+shlex.quote(str(ROOT))]
 native=[Path(json.loads((SOURCE/f'profile-05-{s}/construction.json').read_text())['source']) for s in ['initial','applied']]
 refs=BASE/'texture-fill-round-3/croisement02-tree-11/native-front-preparation/experiment/material-references'
 reference_files=[('Own initial native leaf colors and pixel grain',native[0]),('Own applied native leaf colors and pixel grain',native[1]),('Permitted Leicester southeast cottage tree: leaf material only',refs/'leicester-southeast-cottage-tree.png'),('Permitted Leicester moat bank tree: leaf material only',refs/'leicester-moat-bank-tree.png')]
 for state in ['initial','applied']:
  worker=INPUT/state;d=json.loads((worker/'derivation.json').read_text());asset=f'croisement02-hidden-archer05-climbing-{state}';a=members[asset]
  assert sha(d['source_model'])==a['model_sha256']==d['source_model_sha256'];assert sha(worker/'model.blend')==d['prepared_model_sha256'];assert d['geometry_uv_materials_unchanged'] and all(v['protected_rgba_exact'] for v in d['views'])
  # Recheck every frozen local input before translating approval metadata.
  proof=json.loads((worker/'private-inputs/private-inputs.json').read_text())
  for filename,digest in proof['source_evidence'].items():assert sha(filename)==digest
  for rel,digest in proof['files'].items():assert sha(worker/'private-inputs'/rel)==digest
  mask=np.array(Image.open(worker/'private-inputs/mask.png').convert('RGBA'));count=int((mask[:,:,3]==0).sum());assert count==sum(v['editable_pixels'] for v in d['views'])
  overlay=np.array(Image.open(worker/'private-inputs/input.png').convert('RGBA'));overlay[mask[:,:,3]==0,:3]=[220,65,140];Image.fromarray(overlay).save(worker/'editable-overlay.png')
  review=worker/'input-self-review.json';write(review,dict(status='SELF_REVIEW_PASS_LOCAL_INPUTS; root may independently inspect',model_sha256=d['prepared_model_sha256'],native_source_centers=d['source_opaque_centers'],editable_output_pixels=count,observed_materials_protected=True,protected_rgba_exact=True,scope='Native actual review pixels retained; unknown gray first hits only editable. Source geometry/UV/materials exact. Masks are conservative around antialias edges; final baking must preserve native material slots and repeat all7073 native first hits.',image_sha256={n:sha(worker/'private-inputs'/n) for n in ['input.png','solid.png','mask.png']},api_attempted=False))
  target=DEST/state;target.mkdir();workspace=target/'parent-workspace';workspace.mkdir();(workspace/'model.blend').symlink_to(d['source_model'])
  selection=dict(parent_geometry_revision=a['review_revision'],preparation_model=str(worker/'model.blend'),preparation_state=state);selection_path=target/'preparation-selection.json';write(selection_path,selection)
  bridge=dict(kind='approved-state-endpoint-preparation-derivative',source_user_decision=a,exact_user_text=user['exact_user_text'],source_user_receipt_sha256=sha(APPROVAL),derivation=d,input_self_review_sha256=sha(review),scope='Exact user-approved geometry with private metadata-only preparation. This translates approval identity, not a new user texture decision. Unknown material appearance remains pending.');bridge_path=target/'approval-bridge.json';write(bridge_path,bridge)
  files=[APPROVAL,selection_path,bridge_path,*[p for p in worker.rglob('*') if p.is_file()]];evidence={str(p):dict(path=str(p),sha256=sha(p)) for p in files};identity=dict(asset_id=asset,model_sha256=a['model_sha256'],evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
  item=dict(id=asset,workspace=str(workspace),status='ready-for-user',stored_material_validation='PASS',solid=str(worker/'modified/solid.png'),textured=str(worker/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=a['model_sha256'],evidence=evidence),approval_provenance=bridge,preparation_selection=str(selection_path),**selection)
  manifest=target/'review-manifest.json';decisions=target/'decisions.json';write(manifest,dict(version=1,items=[item]));write(decisions,dict(version=1,decisions=[dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=user['exact_user_text'],revision_sha256=revision,original_gallery_decision=a,translation='Existing exact geometry approval; unchanged geometry/materials/UVs. Generated texture appearance remains pending.')]))
  exp=target/'experiment';prepare(manifest,asset,exp,decisions);references=[]
  for i,(label,path) in enumerate(reference_files):
   file=exp/f'reference-{i}-{path.name}';shutil.copyfile(path,file);references.append(dict(label=label,file=str(file),sha256=sha(file),source='material',asset_id=('croisement02-hidden-archer05-climbing-'+['initial','applied'][i] if i<2 else ['leicester-southeast-cottage-tree','leicester-moat-bank-tree'][i-2]),source_file=str(path),source_sha256=sha(path),role='Material colors, leaf scale and pixel grain only. Ignore gray texture gaps and tree shape/depth. Do not add branches, geometry, silhouettes or background.'))
  write(exp/'auxiliary-references.json',dict(input_sha256=sha(exp/'input.png'),lighting_sha256=sha(exp/'solid.png'),references=references))
  for name in ['input','solid','mask']:assert np.array_equal(np.array(Image.open(exp/f'{name}.png')),np.array(Image.open(worker/'private-inputs'/f'{name}.png')))
  command=['node','level-editor/pipeline/src/refinement/generate-textures.ts',str(exp),'--generate','--provider','openrouter','--prompt-variant','short','--no-mask','--lighting-reference',str(exp/'solid.png'),'--auxiliary-references',str(exp/'auxiliary-references.json'),'--prompt-suffix','Fill only unknown gray climbing leaves and short hidden woody supports. Match the own native yellow-green leaf colors and coarse pixel grain. Supplementary permitted tree references are leaf-material examples only; ignore their gray patches, geometry and canopy shape. Preserve all known source detail, tile cameras, thin leaf silhouettes, holes and geometry. Do not create a duplicate recognizable fork or opaque foliage blocks.']
  commands.append(shlex.join(command));batch.append(dict(asset_id=asset,state=state,experiment=str(exp),command=command,input_sha256=sha(exp/'input.png'),mask_sha256=sha(exp/'mask.png'),solid_sha256=sha(exp/'solid.png'),approval_sha256=sha(exp/'approval.json'),references_sha256=sha(exp/'auxiliary-references.json'),editable_pixels=count))
 write(DEST/'batch.json',dict(status='IMMUTABLE LOCAL INPUTS PREPARED; sandbox DNS unavailable, API not retried',packets=batch,api_attempted=False,publication_allowed=False,texture_approval='pending',post_import_guards=['Preserve every observed material and UV exactly.','Repeat all7073 native first hits and RGB/UV ownership checks.','Review actual8/solid8 and original rock/bank contacts for both endpoints.']))
 (DEST/'generate-manually.sh').write_text('\n'.join(commands)+'\n');print(DEST/'batch.json')
if __name__=='__main__':main()
