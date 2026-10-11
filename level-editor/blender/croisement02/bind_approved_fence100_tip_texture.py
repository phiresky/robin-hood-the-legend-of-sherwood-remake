"""Bind scoped tip geometry approval and supplementary own-fence references."""
from pathlib import Path
import json,hashlib,sys
R=Path.cwd();sys.path.insert(0,str(R/'level-editor/refinement'))
from prepare_texture_packet import prepare
B=R/'level-editor/work/croisement02-refinement';D=B/'approved-fence100-tip-texture-oct11-v2';P=D/'packet';A=B/'restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json';ASSET='croisement02-southwest-field-wattle-fence'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
derivation=json.loads((D/'derivation.json').read_text());assert sha(A)==derivation['approval_sha256'];decision=derivation['original_geometry_decision'];model=Path(decision['model']);assert sha(model)==decision['model_sha256']
for path,digest in derivation['files'].items():assert sha(Path(path))==digest
review=dict(status='PASS',reviewer='Codex',scope='All eight source-only tip views reviewed. Sparse six-pixel original color retained; unknown backs editable. Supplementary original whole-fence context and actual approved fence texture show how to continue the wood. Solid/geometry are frozen approved shapes; original camera top-left.')
write(D/'input-review.json',review);W=D/'worker';W.mkdir(exist_ok=False);(W/'model.blend').symlink_to(model)
paths=[A,D/'input-review.json',D/'derivation.json',D/'source-masks.json',D/'mask-inventory.json',D/'known-tip-six.png',*[p for p in P.rglob('*')if p.is_file()]];evidence={str(p):dict(path=str(p),sha256=sha(p))for p in paths};identity=dict(asset_id=ASSET,model_sha256=decision['model_sha256'],evidence={k:v['sha256']for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
item=dict(id=ASSET,workspace=str(W),status='ready-for-user',stored_material_validation='PASS',solid=str(P/'solid.png'),textured=str(P/'textured.png'),revision=dict(sha256=revision,model_sha256=decision['model_sha256'],evidence=evidence),approval_provenance=dict(original_geometry_decision=decision,receipt=str(A),receipt_sha256=sha(A),scope='Scoped rail-tip approval only. Actual asset name used for packet grouping; render selection is exclusively approved tip. Agent input derivative review is not a new user appearance decision.'))
write(D/'review-manifest.json',dict(version=1,items=[item]));write(D/'decisions.json',dict(version=1,decisions=[dict(asset_id=ASSET,scope='geometry',decision='approved',revision_sha256=revision,exact_user_text='all approved',original_geometry_decision=decision)]));prepare(D/'review-manifest.json',ASSET,D/'experiment',D/'decisions.json');E=D/'experiment';refs=[]
for p,role in [(B/'static-terrain-closure-oct11-v1/fence100-context.png','Original game fence context; match brown weathered rail color and tiny blunt extension. Do not copy ground or foliage.'),(B/'static-terrain-closure-oct11-v1/fence100-tip-candidate-v2/actual-close/textured.png','Retained approved fence wood material style; dark uniform tip itself is unfinished and must be filled. Geometry reference only; no new rails or posts.')]:refs.append(dict(source='material',file=str(p),sha256=sha(p),asset_id=ASSET,role=role))
write(E/'auxiliary-references.json',dict(version=1,input_sha256=sha(E/'input.png'),lighting_sha256=sha(E/'solid.png'),references=refs));print(E)
