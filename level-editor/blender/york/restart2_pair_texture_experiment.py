"""Bind unchanged York geometry approval to reviewed texture-input reproductions."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
PAIR=OUT/'restart2/pair-v16'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
from review_evidence import sha
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('worker',type=Path)
args=parser.parse_args()
worker=args.worker.resolve();output=worker.parent
review=json.loads((worker/'input-review.json').read_text())
visual=worker/'visual-input-review.json'
if json.loads(visual.read_text()).get('status')!='PASS':raise ValueError('Texture inputs need explicit visual review')
asset=review['asset_id'];receipt=PAIR/'approval-batch-v2/user-approval.json'
user=json.loads(receipt.read_text());decision=next(r for r in user['decisions'] if r['asset_id']==asset and r['scope']=='geometry')
if sha(receipt)!=review['user_receipt_sha256'] or decision['model_sha256']!=sha(worker/'model.blend') or decision['decision']!='approved':raise ValueError('Current model lacks exact approval')
if not review['geometry_uv_materials_preserved'] or not review['native_first_cameras_preserved']:raise ValueError('Geometry or cameras changed')
evidence={}
def bind(key,path):
    path=Path(path).resolve(strict=True);evidence[key]={'path':str(path),'sha256':sha(path)}
for path in sorted(worker.rglob('*')):
    if path.is_file():bind('texture-preparation/'+str(path.relative_to(worker)),path)
for path in sorted((PAIR/'source-authority-v1').rglob('*')):
    if path.is_file():bind('source-authority/'+str(path.relative_to(PAIR/'source-authority-v1')),path)
bind('exact-user-approval',receipt);bind('approved-pair-card',PAIR/'approval-batch-v2/ready-candidate-v1.json')
frames=json.loads((worker/'modified/views.json').read_text())
for i,path in enumerate(frames['source_mask_evidence']):bind('source-mask-evidence/'+str(i),path)
identity={'asset_id':asset,'model_sha256':sha(worker/'model.blend'),'evidence':{k:r['sha256']for k,r in evidence.items()}}
revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
translation='Unchanged user-approved model, native-first cameras and reviewed semantic source masks. Derived texture-input reproduction, not a new geometry or texture approval.'
item={'id':asset,'workspace':str(worker),'status':'ready-for-user','stored_material_validation':'PASS',
      'solid':str(worker/'modified/solid.png'),'textured':str(worker/'modified/textured.png'),
      'revision':{'sha256':revision,'model_sha256':identity['model_sha256'],'evidence':evidence},
      'approval_provenance':{'receipt':str(receipt),'sha256':sha(receipt),'exact_user_text':user['answer'],
                             'original_review_revision':decision['review_revision'],'translation':translation}}
record={'asset_id':asset,'scope':'geometry','decision':'approved','exact_user_text':user['answer'],
        'revision_sha256':revision,'source_user_receipt':str(receipt),'translation':translation}
manifest=output/'review-manifest.json';decisions=output/'decisions.json'
for path in [manifest,decisions]:
    if path.exists():raise FileExistsError(path)
manifest.write_text(json.dumps({'version':1,'items':[item]},indent=2)+'\n')
decisions.write_text(json.dumps({'version':1,'decisions':[record]},indent=2)+'\n')
print(json.dumps(prepare(manifest,asset,output/'experiment',decisions),indent=2))
