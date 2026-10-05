"""Translate an immutable explicit geometry approval into shared texture packets."""
import hashlib
import json
import shutil
import sys
from datetime import datetime,timezone
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
from review_evidence import sha
OUT=ROOT/'level-editor/work/croisement01-refinement/restart2'
READY=OUT/'ready-trees18-20-v2'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main(ready=READY, destination='approved-tree-fills-v1', exact_user_text='Both trees approved'):
    archive=read(ready/'archive.json')
    for name,digest in archive['files'].items():
        if sha(ready/name)!=digest:raise ValueError('Changed immutable review file: '+name)
    gallery=read(ready/'gallery/evidence.json');dest=OUT/destination;dest.mkdir(exist_ok=False)
    records=[]
    for approved in archive['assets']:
        asset=approved['asset_id'];w=Path(approved['worker']);g=next(i for i in gallery['items'] if i['id']==asset)
        model_hash=approved['model_sha256'];assert sha(w/'model.blend')==model_hash
        for name,digest in approved['evidence'].items():assert sha(w/name)==digest
        decision=dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=exact_user_text,received_via='Root coordinator relayed exact user message for this immutable gallery',gallery=str(ready/'gallery/index.html'),gallery_review_revision=g['review_revision'],model_sha256=model_hash,archive_sha256=sha(ready/'archive.json'),recorded_at=datetime.now(timezone.utc).isoformat(),texture_approval='pending')
        records.append(decision);case=dest/asset;case.mkdir();write(case/'user-decision.json',decision)
        frozen=case/'approved-workspace';frozen.mkdir();shutil.copy2(w/'model.blend',frozen/'model.blend');shutil.copytree(w/'modified',frozen/'modified')
        for name in ['workspace.json','source-masks.json','projection-layers.json','handoff.json','validation.json']:
            if (w/name).exists():shutil.copy2(w/name,frozen/name)
        evidence={}
        def bind(key,p):evidence[key]=dict(path=str(p.resolve()),sha256=sha(p))
        for p in frozen.rglob('*'):
            if p.is_file():bind('approved-workspace/'+str(p.relative_to(frozen)),p)
        for name in ['inspection/root-review.json','inspection/self-review.json','inspection/actual-materials/evidence.json','inspection/saved-tree-geometry.json','inspection/native-geometry-coverage/report.json']:
            bind('technical/'+name,w/name)
        assert read(w/'validation.json')['status']=='PASS'
        actual=read(w/'inspection/actual-materials/evidence.json');assert actual['model_sha256']==model_hash and actual['actual_sheet_sha256']==sha(w/'inspection/actual-materials/sheet.png')
        geometry=read(w/'inspection/saved-tree-geometry.json');assert geometry['model_sha256']==model_hash
        for mesh in geometry['meshes']:
            if mesh['source_node'].startswith('building-'):assert mesh['nonmanifold_edges']==mesh['degenerate_faces']==0
            else:assert mesh['depth_width_ratio']>=1
        frames=read(frozen/'modified/views.json')
        for n,(path,digest) in enumerate(frames['source_mask_evidence'].items()):
            assert sha(Path(path))==digest;bind('native-mask-'+str(n),Path(path))
        bind('original-gallery-archive',ready/'archive.json');bind('user-decision',case/'user-decision.json')
        bridge=dict(kind='schema-translation-of-existing-user-decision',source_decision=decision,unchanged_model=True,unchanged_source_pixels_cameras_ownership=True,display_derivatives='Labels were added above display sheets only. The unlabelled original sheets and camera manifest were bound in the approved archive.',material_validation_basis='Saved workspace validation PASS; actual saved-material eight views and native overlay inspected by worker and root before user approval; closed observed solids and any inferred crown depth independently checked. Texture appearance remains pending.')
        write(case/'approval-bridge.json',bridge);bind('approval-bridge',case/'approval-bridge.json')
        transport={}
        if frames['tile_size']==[256,256]:
            transport['transport_padding']=dict(version=1,kind='bottom-padding',width=1024,height=640,content_box=dict(left=0,top=0,width=1024,height=512))
            selection=case/'transport-selection.json';write(selection,transport);bind('transport-selection',selection)
            transport['preparation_selection']=str(selection)
        identity=dict(asset_id=asset,model_sha256=model_hash,evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        item=dict(id=asset,workspace=str(frozen),status='ready-for-user',stored_material_validation='PASS',solid=str(frozen/'modified/solid.png'),textured=str(frozen/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=model_hash,evidence=evidence),approval_provenance=bridge,**transport)
        translated=dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],revision_sha256=revision,original_gallery_decision=decision,translation='Exact approved geometry, raw source sheets, cameras and ownership; schema translation only.')
        write(case/'review-manifest.json',dict(version=1,items=[item]));write(case/'decisions.json',dict(version=1,decisions=[translated]));result=prepare(case/'review-manifest.json',asset,case/'experiment',case/'decisions.json');print(json.dumps(result),flush=True)
    write(dest/'user-decisions.json',dict(version=1,exact_user_text=exact_user_text,decisions=records))
if __name__=='__main__':main()
