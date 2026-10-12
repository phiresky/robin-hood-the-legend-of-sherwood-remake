"""Revalidate tree source partition, proposed roots and native state inventory."""
import ast
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/active/croisement01'
R = BASE / 'restart2'
OUT = R / 'tree14-cpu-preflight-v1'
OUT.mkdir(exist_ok=False)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


prep_dir = R / 'tree14-source-preparation-v1'
prep = json.loads((prep_dir / 'preparation.json').read_text())
source = BASE / 'baseline/covered.png'
manifest = json.loads((BASE / 'baseline/masks/manifest.json').read_text())
native = next(r for r in manifest['masks'] if r['index'] == 14)
mask_path = BASE / 'baseline/masks' / native['png']
assert sha(source) == prep['source_sha256']
assert sha(mask_path) == prep['native_mask_sha256']
domain = prep_dir / 'known-trunk-root-domain.png'
deferred = prep_dir / 'deferred-branch-context-domain.png'
assert sha(domain) == prep['known_domain_sha256']
assert sha(deferred) == prep['deferred_domain_sha256']
n, k, d = [np.asarray(Image.open(p).convert('L')) > 0 for p in (mask_path, domain, deferred)]
assert not (k & d).any() and np.array_equal(k | d, n)
assert int(k.sum()) == 7551 and int(d.sum()) == 4781
assert native['obstacle_indices'] == []
recipe = ROOT / 'level-editor/blender/croisement01/restart2_tree14_wood.py'
tree = ast.parse(recipe.read_text())
trace = dict(prep['trace_screen_coordinates'])
for stmt in tree.body:
    if isinstance(stmt, ast.Assign) and len(stmt.targets) == 1:
        target = stmt.targets[0]
        if isinstance(target, ast.Subscript) and isinstance(target.value, ast.Name) and target.value.id == 'trace':
            trace[ast.literal_eval(target.slice)] = ast.literal_eval(stmt.value)
roots = {name: rows for name, rows in trace.items() if name.startswith('root_')}
assert len(roots) == 5
assert not (R / 'tree14-wood-v5').exists(), 'Do not overwrite an existing geometry run'
v4 = R / 'tree14-wood-v4/assets/croisement01-tree-14'
audit = json.loads((v4 / 'inspection/independent-wood-coverage-v2.json').read_text())
assert sha(v4 / 'baseline.blend') == audit['model_sha256']
assert len(audit['missed_interior_pixels']) == 10
bank = R / 'tree05-regional-banks-v18/assets/croisement01-tree-05/model.blend'
assert sha(bank) == '7926500e407950bd66b78d13e0ce65b3829dd2263d7db4ba5c57a7ece5dbc818'
crop = (785, 380, 920, 440)
original = Image.open(source).convert('RGB').crop(crop).resize((1080, 480), Image.Resampling.NEAREST)
overlay = original.copy()
draw = ImageDraw.Draw(overlay)
colors = ['cyan', 'orange', 'magenta', 'white', 'lime']
for (name, rows), color in zip(roots.items(), colors):
    points = [((x - crop[0]) * 8, (y - crop[1]) * 8) for x, y, _ in rows]
    draw.line(points, fill=color, width=2)
    for x, y in points:
        draw.ellipse((x-3, y-3, x+3, y+3), fill=color)
board = Image.new('RGB', (1080, 1000), '#333333')
board.paste(original, (0, 20)); board.paste(overlay, (0, 520))
ImageDraw.Draw(board).text((4, 502), 'Proposed centerlines only; radius, mesh contact and silhouette remain unverified', fill='white')
board.save(OUT / 'root-traces-source.png')
layers_path = BASE / 'source-states/layers.json'
layers = json.loads(layers_path.read_text())
bindings = []
for patch in layers['patches'] + layers['mission_patches']:
    for side in ('old_masks', 'new_masks'):
        for ref in patch['state'][side]:
            if (ref['layer'], ref['index']) == (native['layer'], native['layer_index']):
                bindings.append(dict(patch=patch['id'], side=side))
controls = R / 'native-controls-private-oct11-v2'
control_manifest = json.loads((controls / 'manifest.json').read_text())
for row in control_manifest['resources']:
    assert sha(controls / 'library' / row['path']) == row['sha256'], row['path']
for row in control_manifest['records']:
    assert sha(Path(row['contract'])) == row['sha256'], row['id']
actual = {row['id'] for row in control_manifest['records']}
expected = {row['id'] for row in layers['mission_patches']}
assert actual == expected
report = dict(
    status='CPU_SOURCE_AND_PROPOSAL_PREFLIGHT_PASS_GEOMETRY_PENDING',
    recipe_sha256=sha(recipe), source_sha256=sha(source),
    native_pixels=int(n.sum()), known_pixels=int(k.sum()), deferred_pixels=int(d.sum()),
    partition_exact=True, proposed_roots=roots, previous_audit_model_sha256=audit['model_sha256'],
    remaining_v4_interior_misses_global=[[x+689,y+122] for x,y in audit['missed_interior_pixels']],
    bank_sha256=sha(bank), native_obstacle_links=native['obstacle_indices'],
    direct_patch_mask_bindings=bindings,
    state_inventory=dict(map_patches=len(layers['patches']), mission_patches=len(expected),
                         missions=len({r['mission'] for r in layers['mission_patches']}),
                         checked_resources=len(control_manifest['resources']),
                         native_animation_count=control_manifest['native_animation_count'],
                         status='EXISTING_PRIVATE_BINDINGS_REHASHED_NOT_RUNTIME_VERIFIED'),
    next_heavy_argv=['/usr/bin/blender','--background','--threads','2','--python',str(recipe.relative_to(ROOT)),'--','--geometry-only'],
    blockers=['Unbuilt v5 mesh: ten prior interior misses need fresh ray audit and silhouette/contact review.',
              '4781 mixed upper-branch/source pixels remain explicitly deferred; canopy is not covered by this wood scope.',
              'Regional terrain009 and final source ground ownership are incomplete.',
              'Private native-state contracts do not establish editor controls, actor state parity or canonical integration.',
              'Geometry, final texture, export and normal editor checks remain required.'],
    no_blender_or_browser_launched=True,
)
(OUT / 'report.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report['state_inventory']))
