"""Bounded private bake of the approved Tree02 geometry's reviewed fill."""
import shutil,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent))
from restart2_bake_texture_candidate import run
R=ROOT/'level-editor/work/croisement01-refinement/restart2';case=R/'approved-tree02-fill-v1/croisement01-tree-02';e=case/'experiment';out=case/'baked-v1-luminance'
assert not out.exists()
assert shutil.disk_usage(R).free>=10*1024**3+32*1024**2
available=next(int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))
assert available>=6*1024**3
bpy.context.preferences.filepaths.save_version=0
result=run(e,out,e/'generation-review-v1.json',2.,'best-facing-single',4,'luminance',24.,.4)
assert sum(p.stat().st_size for p in out.rglob('*') if p.is_file() and not p.is_symlink())<=32*1024**2,'Fresh private bake/review32MiBcap'
print('TREE02 PRIVATE BAKE COMPLETE',result['candidate_model_sha256'],flush=True)
