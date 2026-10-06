"""Translate the exact grouped stump wood approval into a guarded private handoff."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from texture_decisions import evidence,fields
from texture_staging import validate_texture_handoff
from review_evidence import sha
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main(kind):
    r=ROOT/'level-editor/work/croisement01-refinement/restart2'
    asset,excluded={'stump64':('croisement01-southwest-broken-stump',1424),'stump69':('croisement01-central-ivy-stump',1106)}[kind]
    case=r/('approved-'+kind+'-wood-fill-v1')/asset
    base=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches'
    user=base/'batch-v8/user-approval.json';gallery=base/'batch-v8/evidence.json'
    assert sha(user)=='d68ce43c77a2a6325c642e452eb135389c253a59e83de3e937928d0c12c7c79e'
    approval=read(user);assert sha(gallery)==approval['evidence_sha256']
    item=next(m for c in read(gallery)['cards'] for m in c['members'] if m['asset_id']==asset)
    member=next(m for d in approval['decisions'] if d['scope']=='texture' for m in d['members'] if m['asset_id']==asset)
    assert member['review_revision']==item['review_revision']
    assert member['model_sha256']==item['model_sha256']
    assert sha(Path(item['model']))==item['model_sha256']
    adapter=Path(item['source_evidence']);assert sha(adapter)==item['source_evidence_sha256']
    for entry in item['images']+item['reports']:
        assert sha(gallery.parent/entry['file'])==entry['sha256']
        assert sha(Path(entry['source']))==entry['sha256']
    e=case/'experiment';b=case/'baked-v1-luminance';actual=b/'actual-review-v1';gen=e/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
    assert sha(b/'worker.blend')==sha(actual/'model.blend')==member['model_sha256']
    root_review=read(actual/'inspection/root-review.json');assert root_review['status']=='scoped texture appearance PASS'
    for name,digest in root_review['files'].items():assert sha(actual/name)==digest
    decision=dict(asset_id=asset,scope=f'WOOD texture only;{excluded}ivy/mixed samples remain excluded',decision='approved',exact_user_text=approval['answer'],model_sha256=member['model_sha256'],gallery_review_revision=member['review_revision'],gallery=str(base/'batch-v8/index.html'),approval_path=str(user),approval_sha256=sha(user),adapter_sha256=sha(adapter))
    target=case/'user-texture-decision.json';assert not target.exists();write(target,decision)
    out=case/'texture-handoff-v1';out.mkdir(exist_ok=False);review=out/'review.json'
    write(review,dict(status='ready-for-user',all_eight_actual_views_inspected=True,baked_model_sha256=member['model_sha256'],actual_sheet_sha256=sha(actual/'inspection/actual-materials/sheet.png'),original_user_decision=decision,root_review=root_review,translation='Exact approved grouped card; technical schema translation only. No new visual decision.'))
    record=dict(id=asset,solid=str(e/'solid.png'),textured=str(actual/'inspection/actual-materials/sheet.png'),source_comparison=str(e/'input.png'),source_comparison_secondary=str(gen/'generated-preserved.png'),source_trace=str(gen/'generated-raw.png'),validation=str(b/'validation.json'),review=str(review))
    paths,hashes=evidence(record);images,reports=fields(record);binding=dict(images={k:hashes[k] for k in images},reports={k:hashes[k] for k in reports})
    translated=dict(asset_id=asset,scope='texture',decision='approved',exact_user_text=approval['answer'],review_revision=hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest(),evidence_paths={k:str(v) for k,v in paths.items()},evidence_sha256=hashes,original_gallery_decision=decision)
    write(out/'decisions.json',dict(version=1,decisions=[translated]))
    h=validate_texture_handoff(case/'review-manifest.json',asset,out/'decisions.json',case/'decisions.json')
    for p in [user,adapter,gallery,target]:h['protected_files'][str(p)]=sha(p)
    write(out/'handoff.json',h);print(out/'handoff.json')
if __name__=='__main__':
    for kind in ['stump64','stump69']:main(kind)
