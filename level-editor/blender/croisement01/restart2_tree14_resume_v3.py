"""Resume interrupted private source projection from the exact completed mesh."""
import json,shutil,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from evidence_io import sha
from refinement_workspace import prepare,modified,validate
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3+512*1024**2
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
base=ROOT/'level-editor/work/active/croisement01';old=base/'restart2/tree14-wood-v3';dest=base/'restart2/tree14-wood-v3-resumed';dest.mkdir(exist_ok=False)
model=old/'assets/croisement01-tree-14/baseline.blend';assert sha(model)=='8397f55fe5620707ee2dd291ac5e36e5d3b8a4c045893624a9dec4a2e32a211a'
bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.preferences.filepaths.save_version=0
shutil.copy2(old/'wood-domain.png',dest/'wood-domain.png');worker=dest/'assets/croisement01-tree-14'
prepare(worker,asset_id='croisement01-tree-14',scene_name='Croisement01 Refinement',collection_name='Croisement01 Working',source_path=base/'baseline/covered.png',grouping_manifest=old/'catalog.json',inventory_path=old/'inventory/inventory.json',review_path=old/'grouping-review.json',source_mask_manifest=old/'source-masks.json',width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05))
validate(worker);modified(worker);(worker/'inspection').mkdir(exist_ok=True)
(dest/'construction.json').write_text(json.dumps(dict(status='PRIVATE_RESUMED_SOURCE_PROJECTION_PENDING_REVIEW',model_sha256=sha(worker/'model.blend'),resumed_geometry_sha256=sha(model),known_bark_pixels=7551,deferred_mixed_pixels=4781,crown_excluded=True,inferred_nonplanar_branches=True,no_gameplay_mutation=True,reason='Prior process ended exit143 during workspace preparation. Exact completed geometry retained; source projection and views regenerated in a new directory.'),indent=2)+'\n')
import render_candidate
sys.argv=['render','--',str(worker)];render_candidate.main()
