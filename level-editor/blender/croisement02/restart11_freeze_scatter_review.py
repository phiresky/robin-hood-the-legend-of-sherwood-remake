"""Freeze the source-art scatter surface decision separately from mound geometry."""
import json,sys,hashlib
from pathlib import Path
sys.path[:0]=[str(Path(__file__).parent),str(Path(__file__).resolve().parents[2]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from build_review_gallery import build

def main():
 base=OUT/'restart11-hiding-mound/scatter-review-v1';base.mkdir(exist_ok=False);worker=OUT/'restart9-hiding-scatter/scatter-surfaces-v2';context=OUT/'restart11-hiding-mound/scatter-slope-contact-v1';manifest=json.loads((worker/'manifest.json').read_text());guard=json.loads((worker/'saved-pixel-guard-v1/report.json').read_text());digest=sha(worker/'model.blend');assert digest==manifest['model_sha256']==guard['model_sha256'];assert guard['status']=='PASS'
 evidence=[worker/'manifest.json',worker/'saved-pixel-guard-v1/report.json',context/'source-contact-sheet.png',context/'report.json',context/'original-source-regions.png',context/'original-source-regions.json']+[worker/f'endpoint-{i:02}-eight.png'for i in [2,13,15,16]]
 root=dict(status='ready-for-user',scope='Source-projected ground/bank appearance only; no initial mound or volumetric settled-leaf claim.',model_sha256=digest,reviewer='/root',finding='Root viewed all four scatter sheets, annotated receiver contact and original source regions. Upright15/16 are defensible as terrain appearance matching the source stamp, with stretch disclosed. Initial mound remains HOLD.',evidence=[dict(path=str(p),sha256=sha(p))for p in evidence]);write_json(base/'root-review.json',root)
 reviews=[]
 for index,label in [(2,'Flat ground scatter'),(13,'Separate net leaf scatter'),(15,'Bank face scatter'),(16,'Bank-foot scatter')]:
  row=manifest['records'][index];reviews.append(dict(id=f'scatter-{index:02}',asset_id=f'croisement02-leaf-scatter-{index:02}',status='Source appearance review',model=str(worker/'model.blend'),model_sha256=digest,textured=str(worker/f'endpoint-{index:02}-eight.png'),context=str(context/'source-contact-sheet.png')if index in [15,16]else str(worker/f'endpoint-{index:02}-eight.png'),validation=str(worker/'saved-pixel-guard-v1/report.json'),review=str(base/'root-review.json'),name=label))
 item=dict(id='croisement02-leaf-scatter-source-surfaces',name='Leaf scatter — original artwork on ground and bank',status='ready-for-user',technical_eligible=True,user_approval=None,review_scope='source-art terrain appearance only',model=str(worker/'model.blend'),model_sha256=digest,endpoint_reviews=reviews,textured=str(worker/'endpoint-02-eight.png'),context=str(context/'source-contact-sheet.png'),source_comparison=str(context/'original-source-regions.png'),source_comparison_label='Original static region / exact final source artwork',review=str(base/'root-review.json'),validation=str(worker/'saved-pixel-guard-v1/report.json'),notes=[
 'Approve the exact native leaf artwork applied to existing ground/bank surfaces for31 hiding-place applied states and one separate net-scatter state.20 unique placement groups share this saved model. Existing terrain geometry is unchanged.',
 'Every eight-view sheet begins with the original game camera. Exact opaque source pixels and alpha are preserved; frontmost surface partitioning prevents doubled paint behind the bank.',
 'The two bank variants intentionally behave as surface artwork, not freestanding leaves.120 pixels in variant15 follow an almost vertical bank face; sampled projection stretching reaches2.06× there and3.49× at variant16. The annotated source/receiver comparison makes that interpretation explicit.',
 'The initial hiding mound, individual leaf thickness, movement, characters, gameplay and mission installation are outside this decision. This uses original artwork; no generated texture was requested.',
 ])
 candidates=base/'review-candidates.json';write_json(candidates,dict(map='Croisement02 leaf scatter source appearance',items=[item],without_packets=[]));build(candidates,base/'gallery');ev=base/'gallery/evidence.json';write_json(base/'ready-candidate-v1.json',dict(status='Root scoped source-appearance PASS; pending grouped user decision',id=item['id'],review_candidates=str(candidates),gallery=str(base/'gallery/index.html'),evidence_sha256=sha(ev),model_sha256=digest,root_review=str(base/'root-review.json'),scope=item['review_scope'],api_calls=0,shared_catalog_modified=False))
 print(base/'ready-candidate-v1.json')
if __name__=='__main__':main()
