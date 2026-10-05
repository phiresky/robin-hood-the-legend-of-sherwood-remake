"""Archive the scoped wall approval and prepare its unchanged texture packet."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
OUT=ROOT/'level-editor/work/croisement03-refinement/restart2'
ASSET='croisement03-southeast-stone-wall'
MODEL='0d93ac6c5898fcb843eeb246a9a4847b34f1d74942f0348af1e09d801b211949'
RECEIPT='3886eb09b2868b98a4d26160110f5db947661efa19bacdf0a0d4e7298abeab00'
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def write(path,data):Path(path).write_text(json.dumps(data,indent=2)+'\n')
def main():
    receipt=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/batch-v6-expanded/user-approval.json'
    assert sha(receipt)==RECEIPT
    approval=json.loads(receipt.read_text())
    decision=next(d for d in approval['decisions'] if d['card_id']=='geometry-'+ASSET)
    member=decision['members'][0]
    assert decision['scope']=='geometry' and member['model_sha256']==MODEL
    frozen=OUT/'geometry-round2-wall10'
    freeze=json.loads((frozen/'freeze.json').read_text())
    for name,digest in freeze['files'].items():assert sha(name)==digest,name
    worker=OUT/'stone-wall-v10/assets'/ASSET
    archive=OUT/'geometry-approval-wall10';archive.mkdir(exist_ok=False)
    for name in ['freeze.json','review-candidates.json','visual-review.json']:
        shutil.copyfile(frozen/name,archive/name)
    shutil.copyfile(receipt,archive/'batch-user-approval.json')
    scope=dict(status='approved',approved_by='user',scope='stone geometry only',exact_user_text=approval['answer'],asset_id=ASSET,model_sha256=MODEL,review_revision=member['review_revision'],batch_receipt_sha256=RECEIPT,excluded=['Context tree16','Ivy and foreground vegetation','Complete terrain and map integration','Texture appearance'])
    write(archive/'decision.json',scope)
    snapshot=archive/'asset';snapshot.mkdir()
    for name in ['model.blend','validation.json']:shutil.copyfile(worker/name,snapshot/name)
    for name in ['modified','inspection']:shutil.copytree(worker/name,snapshot/name)
    assert sha(snapshot/'model.blend')==MODEL
    output=OUT/'texture-wall10'/ASSET;output.mkdir(parents=True,exist_ok=False)
    evidence={}
    def bind(key,path):evidence[key]=dict(path=str(path.resolve()),sha256=sha(path))
    for path in snapshot.rglob('*'):
        if path.is_file():bind('approved/'+str(path.relative_to(snapshot)),path)
    for name in ['decision.json','batch-user-approval.json','freeze.json','visual-review.json']:
        bind('approval/'+name,archive/name)
    frames=json.loads((snapshot/'modified/views.json').read_text())
    for i,(name,digest) in enumerate(sorted(frames['source_mask_evidence'].items())):
        assert sha(name)==digest;bind('source-mask-'+str(i),Path(name))
    identity=dict(asset_id=ASSET,model_sha256=MODEL,evidence={k:v['sha256'] for k,v in evidence.items()})
    revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    item=dict(id=ASSET,workspace=str(snapshot),status='ready-for-user',stored_material_validation='PASS',solid=str(snapshot/'modified/solid.png'),textured=str(snapshot/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=MODEL,evidence=evidence),approval_provenance=scope)
    record=dict(asset_id=ASSET,scope='geometry',decision='approved',exact_user_text=approval['answer'],revision_sha256=revision,original_review_revision=member['review_revision'],original_decision=str(archive/'decision.json'),translation='Unchanged approved seven-part stone geometry, cameras, source pixels and ownership; no ivy, tree or terrain approval.')
    write(output/'review-manifest.json',dict(version=1,items=[item]))
    write(output/'decisions.json',dict(version=1,decisions=[record]))
    result=prepare(output/'review-manifest.json',ASSET,output/'experiment',output/'decisions.json')
    write(output/'prepared.json',result);print(json.dumps(result,indent=2))
if __name__=='__main__':main()
