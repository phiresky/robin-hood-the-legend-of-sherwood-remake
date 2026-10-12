"""Bounded saved-material, root-contact and source-coverage review for Tree14 v16."""
import json
import runpy
import shutil
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from evidence_io import sha
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
r=ROOT/'level-editor/work/croisement01-refinement/restart2/tree14-wood-v16-projected'
w=r/'assets/croisement01-tree-14';model=w/'model.blend';phase=json.loads((r/'phase-modified.json').read_text());before=sha(model);assert before==phase['model_sha256']
import render_candidate
sys.argv=['render','--',str(w)];render_candidate.main()
(r/'phase-render.json').write_text(json.dumps(dict(status='SAVED_VIEWS_COMPLETE_PENDING_VISUAL_REVIEW',model_sha256=before,recipe_sha256=sha(Path(__file__))),indent=2)+'\n')
import restart2_root_contact_detail
sys.argv=['contact','--',str(w),'--all-views','--wood-node','scenery-tree14-wood','--center','850','-718','15','--scale','220']
restart2_root_contact_detail.main()
# The contact helper releases the shared lease. Reacquire through the normal
# queue for the short independent census rather than assuming ownership.
sys.argv=['coverage','--','--tree','14','--workspace',str(w)]
runpy.run_path(str(HERE/'restart2_wood_coverage.py'),run_name='__main__')
assert sha(model)==before
(r/'saved-review-checkpoint.json').write_text(json.dumps(dict(status='RENDERED_AND_MEASURED_PENDING_VISUAL_REVIEW',model_sha256=before,recipe_sha256=sha(Path(__file__))),indent=2)+'\n')
