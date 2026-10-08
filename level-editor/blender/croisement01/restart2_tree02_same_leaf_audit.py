"""Independently verify the saved same-leaf fill and expose remaining coverage gaps."""
import json,shutil,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_slots import acquire
from audit_inferred_gap import run
from render_texture_coverage import inspect
R=ROOT/'level-editor/work/croisement01-refinement/restart2';C=R/'approved-tree02-fill-v1/croisement01-tree-02';B=C/'baked-v2-two-sided-crown';O=C/'baked-v3-same-leaf-crown'
assert shutil.disk_usage(R).free>=10*1024**3+1024**2;assert sum(x.stat().st_size for x in O.rglob('*') if x.is_file() and not x.is_symlink())+1024**2<=32*1024**2;assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2;acquire();result=run(B,O) if '--coverage-only' not in sys.argv else json.loads((O/'independent-inferred-audit.json').read_text())
(O/'independent-inferred-audit.json').write_text(json.dumps(result,indent=2)+'\n');validation=json.loads((O/'validation.json').read_text());(O/'layer-0.json').write_text(json.dumps(validation['layers'][0],indent=2)+'\n');manifest=json.loads((O/'actual-review-v1/inspection/actual-camera-manifest.json').read_text());size=384 if '--coverage-only' in sys.argv else 192;manifest['tile_size']=[size,size]
for v in manifest['views']:v['crop']={'width':size,'height':size}
p=O/('coverage-views-384.json' if '--coverage-only' in sys.argv else 'coverage-views.json');p.write_text(json.dumps(manifest,indent=2)+'\n');result=inspect(p,O,O/('coverage-v2-384' if '--coverage-only' in sys.argv else 'coverage-v1'));assert sum(x.stat().st_size for x in O.rglob('*') if x.is_file() and not x.is_symlink())<=32*1024**2;print(json.dumps({'independent_preservation':'PASS','coverage_views':result['views']}),flush=True)
