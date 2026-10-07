"""Translate exact Batch16 jamb approval into the repository texture packet."""
import hashlib,json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';GEO=WORK/'restart2/gate-geometry-v10';BASE=WORK/'restart2/jamb-texture-inputs-v1';worker=BASE;asset='york-castle-west-gatehouse'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
from review_evidence import sha
approved=json.loads((GEO/'user-geometry-approval.json').read_text());receipt=Path(approved['receipt']);assert sha(receipt)==approved['receipt_sha256'];user=json.loads(receipt.read_text());card=next(c for c in user['cards'] if c['card_id']=='geometry-york-castle-portcullis-and-jamb');member=next(m for m in card['members'] if m['asset_id']==asset+'--portcullis-jamb-return');model=Path(member['model']);assert sha(model)==member['model_sha256'];report=json.loads((BASE/'input-review.json').read_text());assert report['geometry_uv_materials_preserved'];assert json.loads((worker/'visual-input-review.json').read_text())['status']=='PASS'
(worker/'model.blend').symlink_to(model)
evidence={}
for p in sorted(worker.rglob('*')):
 if p.is_file():evidence[str(p.relative_to(worker))]={'path':str(p.resolve()),'sha256':sha(p)}
for key,p in [('approval',receipt),('input-guards',BASE/'input-review.json'),('authority',worker/'authority.json'),('freeze',GEO/'gallery-freeze.json')]:evidence[key]={'path':str(p.resolve()),'sha256':sha(p)}
identity={'asset_id':asset,'model_sha256':sha(model),'evidence':{k:v['sha256'] for k,v in evidence.items()}};revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest();translation='Exact approved building-778-portcullis-jamb-return geometry unchanged; accepted dark native reveal colors. Masonry on unseen faces is an explicit same-arch inference. Original building-778 and all other context excluded from synthesis.'
item={'id':asset,'preparation_state':'covered','workspace':str(worker.resolve()),'status':'ready-for-user','stored_material_validation':'PASS','solid':str((worker/'modified/solid.png').resolve()),'textured':str((worker/'modified/textured.png').resolve()),'revision':{'sha256':revision,'model_sha256':sha(model),'evidence':evidence},'approval_provenance':{'receipt':str(receipt),'sha256':sha(receipt),'exact_user_text':user['exact_user_text'],'original_review_revision':member['review_revision'],'translation':translation}}
decision={'asset_id':asset,'scope':'geometry','decision':'approved','exact_user_text':user['exact_user_text'],'revision_sha256':revision,'source_user_receipt':str(receipt),'translation':translation}
manifest=worker/'review-manifest.json';decisions=worker/'decisions.json';manifest.write_text(json.dumps({'version':1,'items':[item]},indent=2)+'\n');decisions.write_text(json.dumps({'version':1,'decisions':[decision]},indent=2)+'\n');result=prepare(manifest,asset,worker/'experiment',decisions);views=worker/'experiment/views.json';packet=json.loads(views.read_text());assert packet['texture_receiver_object_names']==['building-778-portcullis-jamb-return'];print(json.dumps(result,indent=2))
