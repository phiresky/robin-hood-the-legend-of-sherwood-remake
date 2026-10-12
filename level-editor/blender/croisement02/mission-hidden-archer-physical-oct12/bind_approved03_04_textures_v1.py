"""Translate exact approved pose geometry into the shared texture API contract."""
import hashlib,json,shutil,sys,shlex
from pathlib import Path
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
from PIL import Image
import numpy as np
BASE=ROOT/'level-editor/work/croisement02-refinement/mission-hidden-archer-physical-oct12-v1'
DEST=BASE/'approved03-04-textures-v1'
APPROVAL=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/next23-review-hub-oct12-v1/user-approval.json'
PADDING=dict(version=1,kind='bottom-padding',width=1024,height=640,content_box=dict(left=0,top=0,width=1024,height=512))
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
    batch=json.loads((DEST/'prepared-batch.json').read_text());assert sha(APPROVAL)==batch['user_approval_sha256']
    user=json.loads(APPROVAL.read_text());assert user['status']=='approved';commands=[];results=[]
    refs=ROOT/'level-editor/work/croisement02-refinement/texture-fill-round-3/croisement02-tree-11/native-front-preparation/experiment/material-references'
    for row in batch['rows']:
        worker=Path(row['worker']);d=json.loads((worker/'derivation.json').read_text());assert sha(worker/'model.blend')==d['prepared_model_sha256']==row['model_sha256']
        frames=json.loads((worker/'modified/views.json').read_text());asset=frames['asset_id'];card=d['source_user_decision'];assert card['decision']=='approved'
        source_model=ROOT/d['source_model']['path'];assert sha(source_model)==d['source_model']['sha256']
        for p,h in frames['source_mask_evidence'].items():assert sha(p)==h
        assert d['geometry_uv_material_indices_unchanged'] and all(v['approved_textured_rgba_exact'] for v in d['views'])
        target=DEST/'experiments-v2'/f"{row['profile']:02}-{row['phase']}";target.mkdir(parents=True,exist_ok=False)
        parent=target/'approved-parent';parent.mkdir();(parent/'model.blend').symlink_to(source_model)
        selection=dict(parent_geometry_revision=d['source_model']['sha256'],preparation_model=str(worker/'model.blend'),preparation_state=row['phase'],transport_padding=PADDING)
        selection_path=target/'preparation-selection.json';write(selection_path,selection)
        bridge=dict(kind='approved-state-texture-preparation',source_user_decision=card,exact_user_text=user['user_text'],source_user_receipt_sha256=sha(APPROVAL),derivation=d,scope='Exact approved geometry and source-facing materials retained. Pose visibility and private metadata only; generated appearance remains a separate pending decision. Padding protects a blank bottom strip and never rescales approved pixels.')
        bridge_path=target/'approval-bridge.json';write(bridge_path,bridge)
        files=[APPROVAL,source_model,selection_path,bridge_path,*[p for p in worker.rglob('*') if p.is_file()]]
        evidence={str(p):dict(path=str(p),sha256=sha(p)) for p in files}
        identity=dict(asset_id=asset,model_sha256=sha(source_model),evidence={k:v['sha256'] for k,v in evidence.items()})
        revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        item=dict(id=asset,workspace=str(parent),status='ready-for-user',stored_material_validation='PASS',solid=str(worker/'modified/solid.png'),textured=str(worker/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=sha(source_model),evidence=evidence),approval_provenance=bridge,preparation_selection=str(selection_path),**selection)
        manifest=target/'review-manifest.json';decisions=target/'decisions.json'
        write(manifest,dict(version=1,items=[item]));write(decisions,dict(version=1,decisions=[dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=user['user_text'],revision_sha256=revision,original_gallery_decision=card,translation='Existing explicit geometry approval; unchanged saved geometry, material source and UVs, with conservative source-protection inputs.')]))
        experiment=target/'experiment';result=prepare(manifest,asset,experiment,decisions)
        references=[]
        for name in ['leicester-southeast-cottage-tree','leicester-moat-bank-tree']:
            source=refs/f'{name}.png';target_image=experiment/f'{name}.png';shutil.copyfile(source,target_image)
            references.append(dict(source='material',file=str(target_image),sha256=sha(target_image),asset_id=name,role='Leaf and bark color, pixel grain, and material only. Ignore gray unknown patches. Do not copy tree silhouette, shape, proportions or camera.'))
        write(experiment/'auxiliary-references.json',dict(input_sha256=sha(experiment/'input.png'),lighting_sha256=sha(experiment/'solid.png'),references=references))
        mask=np.asarray(Image.open(experiment/'mask.png'));assert int((mask[:,:,3]==0).sum())==row['editable_pixels']
        command=['node','level-editor/pipeline/src/refinement/generate-textures.ts',str(experiment),'--generate','--provider','openrouter','--prompt-variant','short','--no-mask','--lighting-reference',str(experiment/'solid.png'),'--auxiliary-references',str(experiment/'auxiliary-references.json'),'--prompt-suffix','Fill only the unknown gray backs and narrow edge surfaces of this thin leafy cover and its short woody attachment stems. Match the existing native yellow-green leaves, brown bark and coarse game-art pixel grain. The target is a thin draped leafy cover, not a new tree crown. Preserve the approved silhouettes, tiny holes, fine leaf islands, geometry, cameras and every known source pixel. References supply material examples only; ignore their gray patches and tree proportions. Do not invent new branches or bridge gaps with opaque foliage.']
        commands.append(shlex.join(command));results.append(dict(row,**result,command=command))
    write(DEST/'generation-batch.json',dict(status='INPUTS_BOUND_PENDING_LOCAL_VISUAL_REVIEW',rows=results,texture_approval='pending',canonical_writes=False))
    (DEST/'generate-sequential.sh').write_text('#!/bin/sh\nset -eu\ncd '+shlex.quote(str(ROOT))+'\n'+'\n'.join(commands)+'\n')
    print(DEST/'generation-batch.json')
if __name__=='__main__':main()
