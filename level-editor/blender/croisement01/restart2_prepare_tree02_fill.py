"""Bind approved Tree02 geometry to its frozen source-preserving texture packet."""
import hashlib,json,sys
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from prepare_texture_packet import prepare
from restart2_texture_references import REFERENCES
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
def main():
 asset='croisement01-tree-02';w=R/'tree02-v8/assets'/asset;packet=w/'modified'
 card=R/'ready-tree02-geometry-metadata-v1.json';item=json.loads(card.read_text())['items'][0]
 decision_path=R/'tree02-v8/user-geometry-decision-32-card-v1.json';decision=json.loads(decision_path.read_text())
 assert decision['decision']=='approved' and decision['model_sha256']==item['model_sha256']==sha(w/'model.blend')
 assert sha(Path(decision['batch_approval']))==decision['batch_approval_sha256']
 for path,digest in item['evidence'].items():assert sha(Path(path))==digest,path
 case=R/'approved-tree02-fill-v1'/asset;case.mkdir(parents=True,exist_ok=False)
 paths=[card,decision_path,Path(decision['batch_approval']),w/'model.blend']+[p for p in packet.rglob('*') if p.is_file()]+[w/n for n in ['workspace.json','validation.json','handoff.json'] if (w/n).exists()]
 evidence={str(p):dict(path=str(p),sha256=sha(p)) for p in paths}
 identity=dict(asset_id=asset,model_sha256=item['model_sha256'],evidence={k:v['sha256'] for k,v in evidence.items()})
 revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 translated=dict(id=asset,workspace=str(w),status='ready-for-user',technical_eligible=True,generation_eligible=True,stored_material_validation='PASS',solid=str(packet/'solid.png'),textured=str(packet/'textured.png'),revision=dict(sha256=revision,model_sha256=item['model_sha256'],evidence=evidence),approval_provenance=decision)
 write(case/'review-manifest.json',dict(version=1,items=[translated]))
 write(case/'decisions.json',dict(version=1,decisions=[dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=decision['exact_user_text'],revision_sha256=revision,original_gallery_decision=decision)]))
 prepare(case/'review-manifest.json',asset,case/'experiment',case/'decisions.json')
 e=case/'experiment';folder=e/'material-references';folder.mkdir();refs=[]
 for name,rev,box in REFERENCES:
  source=ROOT/'level-editor/work/leicester-refinement/round-1/texture-review/approved-evidence'/name/rev/'textured.png';target=folder/(name+'-leaf-material.png');Image.open(source).crop(box).save(target)
  refs.append(dict(source='material',file=str(target),sha256=sha(target),asset_id=name,role='Leaf material and palette only. Do not copy layout. Preserve target geometry, thin stems, alpha and all native pixels. Gray source regions are unknown, not texture.',parent_image=str(source),parent_sha256=sha(source),crop_box=list(box),crop_method='Exact pixels, no resizing or recoloring',authorization='User explicitly permitted this named Leicester tree'))
 write(e/'auxiliary-references.json',dict(version=1,input_sha256=sha(e/'input.png'),lighting_sha256=sha(e/'solid.png'),references=refs))
 write(case/'preparation-scope.json',dict(status='Prepared after explicit geometry approval; final appearance pending',approval=decision,receivers=['Tree02 decorative wood','Tree02 inferred crown','Tree02 upper stems'],method='Existing frozen transport views, identical cameras, source ownership buffers preserved; no model or UV edits. Supplementary examples supplied for crown with no native color context.'))
 print(case)
if __name__=='__main__':main()
