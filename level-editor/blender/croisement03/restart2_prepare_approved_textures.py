"""Archive the explicit five-card approval and prepare unchanged texture packets."""
import json,hashlib,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from prepare_texture_packet import prepare
from build_review_gallery import build
OUT=ROOT/'level-editor/work/croisement03-refinement';R=OUT/'restart2'
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,data):Path(p).write_text(json.dumps(data,indent=2)+'\n')
def main():
    frozen=json.loads((R/'approval-round1-freeze.json').read_text());manifest=R/'review-candidates.json';assert sha(manifest)==frozen['manifest_sha256']
    for relative,digest in frozen['gallery_files'].items():assert sha(R/relative)==digest,relative
    items=json.loads(manifest.read_text())['items'];assert len(items)==5
    archive=R/'approval-round1';archive.mkdir(exist_ok=False);shutil.copytree(R/'gallery',archive/'gallery');shutil.copyfile(manifest,archive/'review-candidates.json');shutil.copyfile(R/'approval-round1-freeze.json',archive/'freeze.json')
    decision=dict(status='approved',approved_by='user',scope='geometry',exact_user_text='All five approved',authorization='Exact five frozen gallery cards explicitly approved by the user and relayed by root; texture approval remains separate.',freeze_sha256=sha(archive/'freeze.json'),models=frozen['models'])
    write(archive/'decision.json',decision);prepared=[];decisions=[];newitems=[]
    for item in items:
        asset=item['id'];worker=Path(item['model']).parent;assert sha(worker/'model.blend')==frozen['models'][asset]
        review=json.loads(Path(item['review']).read_text());assert review['model_sha256']==frozen['models'][asset] and review['status']=='ready-for-geometry-review';assert json.loads(Path(item['validation']).read_text())['status']=='PASS'
        snapshot=archive/'assets'/asset;snapshot.mkdir(parents=True)
        for name in ['model.blend','validation.json']:shutil.copyfile(worker/name,snapshot/name)
        for name in ['modified','inspection']:shutil.copytree(worker/name,snapshot/name)
        shutil.copyfile(Path(item['ownership']),snapshot/'ownership.json');write(snapshot/'gallery-item.json',item)
        output=R/'texture-round1'/asset;output.mkdir(parents=True,exist_ok=False)
        evidence={}
        def bind(key,path):evidence[key]=dict(path=str(path.resolve()),sha256=sha(path))
        for path in snapshot.rglob('*'):
            if path.is_file():bind('approved/'+str(path.relative_to(snapshot)),path)
        bind('exact-user-decision',archive/'decision.json');bind('frozen-five-gallery',archive/'freeze.json');bind('frozen-gallery-evidence',archive/'gallery/evidence.json')
        frames=json.loads((snapshot/'modified/views.json').read_text())
        for index,(path,digest) in enumerate(sorted(frames['source_mask_evidence'].items())):
            assert sha(path)==digest;bind('source-mask-'+str(index),Path(path))
        transport={}
        if frames['tile_size']==[256,256]:
            padding=dict(version=1,kind='bottom-padding',width=1024,height=640,content_box=dict(left=0,top=0,width=1024,height=512));selection=output/'transport-selection.json';write(selection,dict(transport_padding=padding));bind('transport-selection',selection);transport=dict(transport_padding=padding,preparation_selection=str(selection))
        identity=dict(asset_id=asset,model_sha256=frozen['models'][asset],evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        adapted=dict(id=asset,workspace=str(snapshot),status='ready-for-user',stored_material_validation='PASS',solid=str(snapshot/'modified/solid.png'),textured=str(snapshot/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=frozen['models'][asset],evidence=evidence),approval_provenance=decision,**transport)
        record=dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],revision_sha256=revision,original_five_card_decision=str(archive/'decision.json'),translation='Unchanged approved geometry, source pixels, cameras and ownership. Bottom transport padding where needed changes no reviewed content.')
        write(output/'review-manifest.json',dict(version=1,items=[adapted]));write(output/'decisions.json',dict(version=1,decisions=[record]));result=prepare(output/'review-manifest.json',asset,output/'experiment',output/'decisions.json');prepared.append(result);decisions.append(record)
        item['user_approval']='approved';item['approval_decision']=str(archive/'decision.json');newitems.append(item)
    write(R/'geometry-decisions-round1.json',dict(version=1,decisions=decisions));write(R/'texture-round1/prepared.json',prepared)
    approved_manifest=R/'round1-approved-gallery-manifest.json';write(approved_manifest,dict(map='Croisement03',items=newitems,status_counts={'user geometry approved':5},scope='Five isolated geometry candidates approved. Texture, surrounding scene and final integration remain unfinished.'));build(approved_manifest,R/'gallery',pending_only=True)
    print(json.dumps(prepared,indent=2))
if __name__=='__main__':main()
