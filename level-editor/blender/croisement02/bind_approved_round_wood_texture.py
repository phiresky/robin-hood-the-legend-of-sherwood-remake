"""Bind user-approved geometry to explicitly reviewed source-only fill inputs."""
from pathlib import Path
import sys,json,hashlib,shutil
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(Path(__file__).parent)]
from prepare_texture_packet import prepare
B=R/'level-editor/work/croisement02-refinement';A=B/'restart3-review-batches/three-tree-geometry-oct11-v1/user-approval.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):return json.loads(p.read_text())
def write(p,d):p.write_text(json.dumps(d,indent=2)+'\n')
for n in map(int,sys.argv[1:]):
 D=B/f'approved-round-wood-textures-oct11-v1/tree{n}';P=D/'packet';derivation=read(P/'derivation.json');assert sha(A)==derivation['user_geometry_approval_sha256'];decision=derivation['original_geometry_decision'];model=Path(decision['model']);assert sha(model)==decision['model_sha256']
 for path,digest in derivation['artifacts'].items():assert sha(P/path)==digest
 review=dict(reviewer='Codex',status='PASS',ready_for_texture_preparation=True,derivation_sha256=sha(P/'derivation.json'),views_checked=16,scope='All eight source-only and solid views checked; approved geometry and camera transforms unchanged. Wood scope excludes crown/ground. Shaded unknown surfaces are editable; known source pixels and alpha protected.');write(D/'input-review.json',review)
 W=D/'worker';W.mkdir(exist_ok=False);(W/'model.blend').symlink_to(model)
 paths=[A,D/'input-review.json',*[p for p in P.rglob('*')if p.is_file()]];evidence={str(p):dict(path=str(p),sha256=sha(p))for p in paths};identity=dict(asset_id=decision['asset_id'],model_sha256=decision['model_sha256'],evidence={k:v['sha256']for k,v in evidence.items()});revision=hashlib.sha256(json.dumps(identity,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 item=dict(id=decision['asset_id'],workspace=str(W),status='ready-for-user',stored_material_validation='PASS',solid=str(P/'solid.png'),textured=str(P/'textured.png'),revision=dict(sha256=revision,model_sha256=decision['model_sha256'],evidence=evidence),approval_provenance=dict(original_geometry_decision=decision,receipt=str(A),receipt_sha256=sha(A),source_derivation=derivation,agent_input_review=review,scope='User approved unchanged geometry. Source-only wood input derivative is agent-reviewed preparation, not a new user appearance decision.'))
 write(D/'review-manifest.json',dict(version=1,items=[item]));write(D/'decisions.json',dict(version=1,decisions=[dict(asset_id=decision['asset_id'],scope='geometry',decision='approved',revision_sha256=revision,exact_user_text=read(A)['user_message'],original_geometry_decision=decision,translation='Explicit approved geometry with agent-reviewed source-only preparation derivative; texture approval remains pending')]))
 result=prepare(D/'review-manifest.json',decision['asset_id'],D/'experiment',D/'decisions.json');E=D/'experiment';refs=read(B/'restart6-tree07-bark-close-v1/auxiliary-references.json')
 for ref in refs['references']:
  path=Path(ref['file']);assert sha(path)==ref['sha256'];assert ref['asset_id']in ['leicester-southeast-cottage-tree','leicester-moat-bank-tree']
 refs.update(input_sha256=sha(E/'input.png'),lighting_sha256=sha(E/'solid.png'));write(E/'auxiliary-references.json',refs);print(result)
