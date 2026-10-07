"""Translate the exact approved isolated wood appearance for private integration."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from texture_decisions import evidence,fields
from texture_staging import validate_texture_handoff
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 case=R/'approved-tree01-isolated-wood-fill-v1/croisement01-tree-01';approval_path=case/'user-texture-decision-32-card-v1.json';approval=read(approval_path)
 assert approval['decision']=='approved'
 assert sha(Path(approval['batch_approval']))==approval['batch_approval_sha256']
 for p,h in approval['rehashed_evidence'].items():assert sha(Path(p))==h,p
 b=case/'baked-v1-luminance';e=case/'experiment';actual=b/'actual-review-v1'
 assert sha(b/'worker.blend')==approval['model_sha256']
 root_review=actual/'inspection/root-review-v17.json';assert read(root_review)['status'].startswith('PASS scoped Texture')
 out=case/'texture-handoff-32-card-v1';out.mkdir(exist_ok=False)
 review=out/'review.json'
 write(review,dict(status='ready-for-user',all_eight_actual_views_inspected=True,baked_model_sha256=approval['model_sha256'],actual_sheet_sha256=sha(actual/'inspection/actual-materials/sheet.png'),original_user_decision=approval,original_user_decision_sha256=sha(approval_path),root_review=read(root_review),translation='Exact user-approved isolated WOOD texture. Original crown and approved geometry unchanged. Full assembly occlusion and private integration checks remain mandatory.'))
 gen=e/'generation-short-no-mask-with-lighting-openrouter-with-auxiliary'
 record=dict(id=approval['asset_id'],solid=str(e/'solid.png'),textured=str(actual/'inspection/actual-materials/sheet.png'),source_comparison=str(e/'input.png'),source_comparison_secondary=str(gen/'generated-preserved.png'),source_trace=str(gen/'generated-raw.png'),validation=str(b/'validation.json'),review=str(review))
 paths,hashes=evidence(record);images,reports=fields(record)
 binding=dict(images={k:hashes[k] for k in images},reports={k:hashes[k] for k in reports})
 decision=dict(asset_id=approval['asset_id'],scope='texture',decision='approved',exact_user_text=approval['exact_user_text'],review_revision=hashlib.sha256(json.dumps(binding,sort_keys=True).encode()).hexdigest(),evidence_paths={k:str(v) for k,v in paths.items()},evidence_sha256=hashes,original_gallery_decision=approval,translation='Approved image content and model unchanged; technical schema translation only.')
 write(out/'decisions.json',dict(version=1,decisions=[decision]))
 handoff=validate_texture_handoff(case/'review-manifest.json',approval['asset_id'],out/'decisions.json',case/'decisions.json')
 for p in [approval_path,root_review,Path(approval['review_metadata']),Path(approval['batch_approval'])]:handoff['protected_files'][str(p)]=sha(p)
 write(out/'handoff.json',handoff);print(out/'handoff.json')
if __name__=='__main__':main()
