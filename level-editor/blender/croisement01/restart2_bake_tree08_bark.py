"""Bake the inspected Tree08 bark candidate under the shared resource guards."""
import shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire
from restart2_bake_texture_candidate import run
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';E=W/'experiment'
acquire(slots=2)
assert shutil.disk_usage(R).free>=10*1024**3
assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
run(E,W/'baked-v1',E/'generation-review-v1.json',texels_per_unit=2,reconciliation_gain_mode='luminance')
