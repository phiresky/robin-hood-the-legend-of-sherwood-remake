"""Bind explicit wagon geometry approval to unchanged source preparation."""
import sys,json,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from prepare_texture_packet import prepare
O=ROOT/'level-editor/work/croisement02-refinement';D=O/'restart4-south-cart-texture/initial-source-inputs-v1';B=D.parent/'approved-fill-v1';B.mkdir()
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
A=O/'restart3-review-batches/batch-v9/user-approval.json';a=json.loads(A.read_text());asset='croisement02-south-cart-initial-physical';member=next(m for c in a['decisions'] for m in c['members'] if m['asset_id']==asset);der=json.loads((D/'derivation.json').read_text());assert der['geometry_uv_materials_unchanged'] and member['model_sha256']==der['source_model_sha256']
write(D/'self-review.json',dict(reviewer='approved_texture_integration',ready_for_texture_preparation=True,prepared_model_sha256=sha(D/'model.blend'),images={str(D/'modified'/f):sha(D/'modified'/f) for f in ['textured.png','solid.png']},findings=['All eight views personally inspected; native camera first, full uncut wagon, no horse artwork','Separate roof straw, platform and cabin wood native masks; unknown wheel/frame surfaces neutral','Approved geometry UV and materials unchanged; native source projection uses guarded first-hit ownership']))
parent=B/'parent-workspace';parent.mkdir();shutil.copyfile(der['source_model'],parent/'model.blend')
selection=dict(parent_geometry_revision=member['review_revision'],preparation_model=str(D/'model.blend'));write(B/'preparation-selection.json',selection)
original=dict(member,scope='geometry',decision='approved',model=der['source_model'])
bridge=dict(kind='approved-state-endpoint-preparation-derivative',source_user_decision=original,source_decisions_sha256=sha(A),derivation=der,scope='Exact user-approved wagon geometry/UV/materials unchanged; only ownership metadata and source-only review preparation added. No texture approval inferred.')
write(B/'approval-bridge.json',bridge)
paths=[A,B/'preparation-selection.json',B/'approval-bridge.json']+[p for p in D.rglob('*') if p.is_file()]
evidence={str(p):dict(path=str(p),sha256=sha(p)) for p in paths};identity=dict(asset_id=asset,model_sha256=member['model_sha256'],evidence={k:v['sha256'] for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
item=dict(id=asset,workspace=str(parent),status='ready-for-user',stored_material_validation='PASS',solid=str(D/'modified/solid.png'),textured=str(D/'modified/textured.png'),revision=dict(sha256=revision,model_sha256=member['model_sha256'],evidence=evidence),approval_provenance=bridge,preparation_selection=str(B/'preparation-selection.json'),**selection)
write(B/'review-manifest.json',dict(version=1,items=[item]));write(B/'decisions.json',dict(version=1,decisions=[dict(asset_id=asset,scope='geometry',decision='approved',exact_user_text=a['answer'],revision_sha256=revision,original_gallery_decision=original,translation='Exact approved geometry, independently guarded unchanged preparation; no appearance approval inferred')]))
print(prepare(B/'review-manifest.json',asset,B/'experiment',B/'decisions.json'))

# References contain only disjoint wagon material pixels, never the full actor sprite.
from PIL import Image
E=B/'experiment';S=O/'restart3-south-cart/initial-physical-v8';refs=[]
for role,description in [('roof','Own native golden straw thatch roof, fine parallel straw bundles; only curved canopy gets straw'),('fore_platform','Own native tan rough timber front platform, aged irregular grain; no new ropes or animals'),('cabin_walls','Own native dark warm timber cabin boards; wheels and frames should use coherent weathered wood, never straw')]:
 im=Image.open(S/f'{role}-source.png').convert('RGBA');im=im.crop(im.getchannel('A').getbbox());im=im.resize((im.width*4,im.height*4),Image.Resampling.NEAREST);p=E/f'reference-{role}.png';im.save(p);refs.append(dict(file=p.name,sha256=sha(p),source='material',asset_id=asset,role=description))
write(E/'auxiliary-references.json',dict(input_sha256=sha(E/'input.png'),lighting_sha256=sha(E/'solid.png'),references=refs))
