"""Resume the interrupted private tree preparation from its saved geometry."""
import json,sys
from pathlib import Path
import bpy
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import OUT
from refinement_workspace import prepare,modified,validate
from render_slots import acquire
from evidence_io import sha
R=OUT/'restart2';source=R/'tree04-v5';dest=R/'tree04-v5-recovered';assert not dest.exists();assert (source/'interrupted-build.json').exists();baseline=source/'assets/croisement01-tree-04/baseline.blend';assert baseline.exists();acquire();bpy.ops.wm.open_mainfile(filepath=str(baseline));bpy.context.preferences.filepaths.save_version=0;dest.mkdir();worker=dest/'assets/croisement01-tree-04'
prepare(worker,asset_id='croisement01-tree-04',scene_name='Croisement01 Refinement',collection_name='Croisement01 Working',source_path=OUT/'baseline/covered.png',grouping_manifest=source/'catalog.json',inventory_path=source/'inventory/inventory.json',review_path=source/'grouping-review.json',source_mask_manifest=source/'source-masks.json',width=384,height=384,framing_padding=1.2,lighting=dict(toward_sun=[-.6,-.4,.7],ambient=.22,diffuse=.78,shadow_epsilon=.05));validate(worker);modified(worker)
(dest/'recovery.json').write_text(json.dumps(dict(status='Recovered preparation; unapproved geometry',interrupted_session=78401,source_baseline_sha256=sha(baseline),model_sha256=sha(worker/'model.blend'),construction_recipe='restart2_tree04.py revision5',preserved_interrupted_outputs=True),indent=2)+'\n');import render_candidate;sys.argv=['render','--',str(worker)];render_candidate.main()
