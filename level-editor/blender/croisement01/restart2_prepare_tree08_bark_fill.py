"""Bind Tree08 approved wood geometry and its exact semantic bark ownership to synthesis."""
import hashlib,json,sys
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from prepare_texture_packet import prepare
from restart2_texture_references import REFERENCES
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08'
A=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/three-tree-geometry-oct11-v1/user-approval.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
def main():
 decision=json.loads(A.read_text());member=next(d for d in decision['decisions'] if d['asset_id']=='croisement01-tree-08')
 assert member['decision']=='approved' and member['scope']=='geometry' and member['model_sha256']==sha(W/'model.blend')
 proof=json.loads((W/'derivation-proof.json').read_text());assert proof['status']=='PASS_EXACT_APPROVED_GEOMETRY_TRANSPORT'
 files=[A]+[p for p in W.rglob('*') if p.is_file()]
 evidence={str(p):{'path':str(p),'sha256':sha(p)} for p in files}
 identity={'asset_id':member['asset_id'],'model_sha256':sha(W/'model.blend'),'evidence':{k:v['sha256'] for k,v in evidence.items()}}
 revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 provenance={'exact_user_text':decision['user_message'],'approval_path':str(A),'approval_sha256':sha(A),'geometry':member,'derivation':proof,'texture_approval':'pending'}
 item={'id':member['asset_id'],'workspace':str(W),'status':'ready-for-user','technical_eligible':True,'generation_eligible':True,'stored_material_validation':'PASS','solid':str(W/'modified/solid.png'),'textured':str(W/'modified/textured.png'),'preparation_model':str(W/'transport-model.blend'),'revision':{'sha256':revision,'model_sha256':sha(W/'model.blend'),'evidence':evidence},'approval_provenance':provenance}
 write(W/'review-manifest.json',{'version':1,'items':[item]});write(W/'decisions.json',{'version':1,'decisions':[{'asset_id':member['asset_id'],'scope':'geometry','decision':'approved','exact_user_text':decision['user_message'],'revision_sha256':revision,'original_gallery_decision':provenance}]})
 prepare(W/'review-manifest.json',member['asset_id'],W/'experiment',W/'decisions.json')
 E=W/'experiment';folder=E/'material-references';folder.mkdir();refs=[]
 for name,rev,_ in REFERENCES:
  source=ROOT/'level-editor/work/leicester-refinement/round-1/texture-review/approved-evidence'/name/rev/'textured.png'
  target=folder/(name+'-native-example.png');Image.open(source).crop((0,0,256,320)).save(target)
  refs.append({'source':'material','file':str(target),'sha256':sha(target),'asset_id':name,'role':'Supplementary native-view tree example. Use only the brown trunk bark character and bark palette; do not add foliage to the target wood, and do not copy geometry, gray patches, background or proportions. Target source bark has priority.','parent_image':str(source),'parent_sha256':sha(source),'crop_box':[0,0,256,320],'crop_method':'Exact first native camera tile pixels; no resizing or recoloring','authorization':'Only the two Leicester trees explicitly permitted by user.'})
 write(E/'auxiliary-references.json',{'version':1,'input_sha256':sha(E/'input.png'),'lighting_sha256':sha(E/'solid.png'),'references':refs})
 print(E)
if __name__=='__main__':main()
