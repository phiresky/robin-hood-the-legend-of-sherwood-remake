"""Bind unchanged York geometry approval to reviewed texture-input reproductions."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'level-editor/work/york-refinement'
HALL=OUT/'restart2/hall-return-four-states-v1'
AUTHORITY=OUT/'restart2/hall-source-authority-v2'
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
asset=review['asset_id'];receipt=HALL/'approval-batch-v8/user-approval.json'
user=json.loads(receipt.read_text());card=next(r for r in user['decisions'] if r['card_id']=='geometry-york-castle-great-hall-return-four-states-v2' and r['scope']=='geometry');decision=next(r for r in card['members'] if r['asset_id']==review['decision_id'])
if sha(receipt)!=review['user_receipt_sha256'] or decision['model_sha256']!=sha(worker/'model.blend'):raise ValueError('Current model lacks exact approval')
if not review['geometry_uv_materials_preserved'] or not review['native_first_cameras_preserved']:raise ValueError('Geometry or cameras changed')
evidence={}
def bind(key,path):
    path=Path(path).resolve(strict=True);evidence[key]={'path':str(path),'sha256':sha(path)}
for path in sorted(worker.rglob('*')):
    if path.is_file():bind('texture-preparation/'+str(path.relative_to(worker)),path)
for path in sorted(AUTHORITY.rglob('*')):
    if path.is_file():bind('source-authority/'+str(path.relative_to(AUTHORITY)),path)
bind('exact-user-approval',receipt);bind('approved-hall-four-state-card',HALL/'ready-candidate-v2.json')
frames=json.loads((worker/'modified/views.json').read_text())
for i,path in enumerate(frames['source_mask_evidence']):bind('source-mask-evidence/'+str(i),path)
identity={'asset_id':asset,'model_sha256':sha(worker/'model.blend'),'evidence':{k:r['sha256']for k,r in evidence.items()}}
revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
translation='Unchanged user-approved model, native-first cameras and reviewed semantic source masks. Derived state texture-input reproduction; original geometry decision ID is ' + review['decision_id'] + '. No new geometry or texture approval.'
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
# The verified model remains an immutable source reference instead of another full-scene copy.
import prepare_texture_packet as preparation
copy_file=preparation.shutil.copyfile
model=(worker/'model.blend').resolve(strict=True)
def reference_model(source,destination,*args,**kwargs):
 if Path(destination).name=='approved-model.blend':
  if Path(source).resolve(strict=True)!=model:raise ValueError('Unexpected approved model source')
  Path(destination).symlink_to(model);return str(destination)
 return copy_file(source,destination,*args,**kwargs)
preparation.shutil.copyfile=reference_model
try:result=prepare(manifest,asset,output/'experiment',decisions)
finally:preparation.shutil.copyfile=copy_file
views=output/'experiment/views.json';data=json.loads(views.read_text());scope=['Castle great hall / Structural volume 791','Castle great hall / Northwest stone arch'];assert set(scope)<=set(data['object_names']);data['texture_receiver_object_names']=scope;data['bounded_correction_scope']='791 and arch only; other existing materials must remain byte-identical';views.write_text(json.dumps(data,indent=2)+'\n')
print(json.dumps(result,indent=2))
