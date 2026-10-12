"""Run bounded, resumable review phases on the completed tree geometry."""
import argparse
import json
import shutil
import sys
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from evidence_io import sha
from render_slots import acquire
from refinement_workspace import prepare, modified, validate

p = argparse.ArgumentParser()
p.add_argument('phase', choices=['prepare', 'modified', 'render'])
p.add_argument('--revision', type=int, default=5)
a = p.parse_args(sys.argv[sys.argv.index('--')+1:])
acquire(slots=2)
assert shutil.disk_usage(ROOT).free > 10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024 > 6*1024**3
base = ROOT/'level-editor/work/active/croisement01'
assert a.revision >= 5
old = base/f'restart2/tree14-wood-v{a.revision}'
dest = base/f'restart2/tree14-wood-v{a.revision}-projected'
worker = dest/'assets/croisement01-tree-14'
checkpoint = json.loads((old/'geometry-checkpoint.json').read_text())
baseline = old/'assets/croisement01-tree-14/baseline.blend'
assert sha(baseline) == checkpoint['baseline_sha256']
if a.phase == 'prepare':
    dest.mkdir(exist_ok=False)
    shutil.copy2(old/'wood-domain.png', dest/'wood-domain.png')
    bpy.ops.wm.open_mainfile(filepath=str(baseline))
    bpy.context.preferences.filepaths.save_version = 0
    prepare(worker, asset_id='croisement01-tree-14', scene_name='Croisement01 Refinement',
            collection_name='Croisement01 Working', source_path=base/'baseline/covered.png',
            grouping_manifest=old/'catalog.json', inventory_path=old/'inventory/inventory.json',
            review_path=old/'grouping-review.json', source_mask_manifest=old/'source-masks.json',
            width=384, height=384, framing_padding=1.2,
            lighting=dict(toward_sun=[-.6,-.4,.7], ambient=.22, diffuse=.78, shadow_epsilon=.05))
elif a.phase == 'modified':
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'))
    validate(worker)
    modified(worker)
else:
    (worker/'inspection').mkdir(exist_ok=True)
    import render_candidate
    sys.argv = ['render', '--', str(worker)]
    render_candidate.main()
(dest/f'phase-{a.phase}.json').write_text(json.dumps(dict(
    status='PHASE_COMPLETE_NOT_VISUAL_APPROVAL', phase=a.phase,
    baseline_sha256=checkpoint['baseline_sha256'], model_sha256=sha(worker/'model.blend'),
    recipe_sha256=sha(Path(__file__)),
), indent=2)+'\n')
