"""Export the exact approved tree02 privately, checking both approval scopes."""
import json
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire, release
from texture_staging import verify_baked_geometry
from export_editor import export_asset_library
from review_evidence import sha

R = ROOT/'level-editor/work/active/croisement01/restart2'
asset = 'croisement01-tree-02'
receipt = ROOT/'level-editor/work/active/croisement02/restart3-review-batches/next-tree02-channels-mounds-v1/user-approval.json'
approval = json.loads(receipt.read_text())
assert approval['status'] == 'USER_APPROVED'
decision = next(d for d in approval['decisions'] if d['asset_id'] == asset)
assert decision['scope'] == 'texture'
model = Path(decision['model'])
assert sha(model) == decision['model_sha256'] == '0436e5e2ad8ca3109fc2aeeaafa223f2d0c90cb5eea694bec0dcd2409ea1408f'
workspace = R/'tree02-v8/assets'/asset
config = json.loads((workspace/'workspace.json').read_text())
grouping = workspace/'reference/grouping.json'
assert sha(grouping) == config['grouping_manifest_sha256']
geometry_decision = json.loads((R/'approved-tree02-fill-v1'/asset/'decisions.json').read_text())['decisions'][0]
assert geometry_decision['decision'] == 'approved'
source = workspace/'model.blend'
source_sha = geometry_decision['original_gallery_decision']['model_sha256']
assert sha(source) == source_sha
handoff = dict(asset_id=asset, scene_name=config['scene_name'], collection_name=config['collection_name'],
    object_names=['Tree02 decorative wood', 'Tree02 upper stems', 'Tree02 inferred crown'],
    source_nodes=config['part_ids'], projection_kind='source-projected',
    approved_source_blend=str(source), blend_path=str(model), blend_sha256=sha(model),
    protected_files={str(source): source_sha, str(model): sha(model), str(receipt): sha(receipt)})
acquire(slots=2)
try:
    assert shutil.disk_usage(R).free >= 10*1024**3
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')) >= 6*1024**2
    proof = verify_baked_geometry(handoff)
    out = R/'tree02-approved-integration-v1'
    out.mkdir(exist_ok=False)
    catalog = json.loads(grouping.read_text())
    report = export_asset_library('Croisement01', out/'assets', ROOT/'level-editor/work/active/croisement01/baseline/Croisement01.rhp.json', asset_ids=[asset], catalog=catalog)
    assert sha(model) == decision['model_sha256']
    files = {str(p.relative_to(out)): sha(p) for p in out.rglob('*') if p.is_file()}
    (out/'export-proof.json').write_text(json.dumps(dict(scope='Private approved tree02 export; no canonical writes', geometry=proof, export=report, approval_sha256=sha(receipt), grouping_sha256=sha(grouping), files=files), indent=2)+'\n')
    print(out, flush=True)
finally:
    release()
