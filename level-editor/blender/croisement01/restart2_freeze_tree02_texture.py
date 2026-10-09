"""Freeze the self-reviewed saved texture for one grouped approval card."""
import hashlib
import json
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
B = R / 'approved-tree02-fill-v1/croisement01-tree-02/baked-v6-filtered-leaf-gaps'
O = R / 'ready-tree02-texture-v3'
assert not O.exists()
assert shutil.disk_usage(R).free >= 10 * 1024**3 + 32 * 1024**2
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
receipt = json.loads((B / 'reopened-preservation.json').read_text())
coverage = json.loads((B / 'coverage-v1-384/coverage.json').read_text())
render = json.loads((B / 'actual-review-v1/inspection/actual-materials/evidence.json').read_text())
assert receipt['model_sha256'] == coverage['model_sha256'] == render['model_sha256'] == sha(B / 'worker.blend')
assert all(v['unfilled_visible_pixels'] == 0 for v in coverage['views']) and not coverage['unverified_materials']
assert receipt['all_source_rgba_unchanged'] and receipt['all_alpha_unchanged'] and receipt['all_other_pixels_images_geometry_uv_unchanged']
O.mkdir()
files = {
    'model.blend': B / 'worker.blend',
    'textured.png': B / 'actual-review-v1/inspection/actual-materials/sheet.png',
    'source-comparison.png': B / 'actual-review-v1/inspection/native-source/comparison.png',
    'coverage.png': B / 'coverage-v1-384/coverage.png',
    'coverage.json': B / 'coverage-v1-384/coverage.json',
    'saved-preservation.json': B / 'reopened-preservation.json',
    'validation.json': B / 'validation.json',
    'views.json': B / 'actual-review-v1/inspection/actual-camera-manifest.json',
}
for i in range(8):
    files[f'view-{i}.png'] = B / f'actual-review-v1/inspection/actual-materials/view-{i}-textured.png'
for name, source in files.items():
    shutil.copy2(source, O / name)
    assert sha(source) == sha(O / name)
review = dict(status='PASS_SELF_REVIEW_READY_FOR_GROUPED_TEXTURE_APPROVAL',
              model_sha256=receipt['model_sha256'], inspected=['all eight saved actual-material views', 'native source / actual / overlay', 'all eight provenance diagnostic views'],
              judgment='Narrow stems remain continuously bark-textured in every view; the inferred crown has consistent golden foliage. Native source alignment is retained. No conspicuous gray fill holes remain.',
              technical='All eight diagnostic red-pixel counts are zero; no unverified materials. Exact alpha, source RGBA, geometry and UV preservation is checked after reopening each repaired save.',
              limitations=['Eight camera diagnostics do not establish coverage of every unseen underside.', 'The narrow lower-stem junction retains its approved geometry and coarse source colors.', 'Crown and unseen bark color are inferred, not new observed source.'],
              source_ownership_expansion=False, canonical_writes=False)
(O / 'self-review.json').write_text(json.dumps(review, indent=2) + '\n')
archive = {p.name: sha(p) for p in O.iterdir() if p.is_file()}
(O / 'archive.json').write_text(json.dumps(archive, indent=2) + '\n')
handoff = dict(asset_id='croisement01-tree-02', map='Croisement01', scope='texture appearance only',
               status='READY_FOR_GROUPED_USER_REVIEW', model=str((O / 'model.blend').resolve()), model_sha256=receipt['model_sha256'],
               textured=str((O / 'textured.png').resolve()), source_comparison=str((O / 'source-comparison.png').resolve()),
               coverage=str((O / 'coverage.png').resolve()), native_camera_top_left=True,
               gallery=str((O / 'index.html').resolve()), archive=str((O / 'archive.json').resolve()), archive_sha256=sha(O / 'archive.json'),
               self_review=str((O / 'self-review.json').resolve()), geometry_approval=str((B.parent / 'decisions.json').resolve()),
               bake_workspace=str((B / 'actual-review-v1').resolve()), validation=str((B / 'validation.json').resolve()),
               decision='pending; collect with other cards', inference='Saved generated bark and crown plus bounded same-surface edge continuation; original pixels and alpha preserved.')
(O / 'handoff.json').write_text(json.dumps(handoff, indent=2) + '\n')
(O / 'index.html').write_text('''<!doctype html><meta charset="utf-8"><title>Crossings01 tree02 texture</title>
<style>body{background:#202124;color:#eee;font:16px system-ui;max-width:1500px;margin:2rem auto}img{max-width:100%}a{color:#9df}</style>
<h1>Crossings01 tree02 — texture appearance</h1>
<p>Pending grouped approval. Geometry is already approved. Original camera is top-left.</p>
<p>Inferred bark and golden foliage fill the unseen surfaces. Original pixels and alpha are preserved; small edge gaps continue colors from the same bark sleeve or physical leaf.</p>
<a href="textured.png"><img src="textured.png" alt="Eight saved-model material views"></a>
<h2>Original artwork, saved model and overlay</h2><a href="source-comparison.png"><img src="source-comparison.png" alt="Native source comparison"></a>
<p>The narrow lower-stem junction retains its approved shape and coarse source colors.</p>
<details><summary>Coverage evidence</summary><img src="coverage.png" alt="Source green, generated blue, inferred continuation yellow"><p>Zero red diagnostic pixels in all eight views; this does not prove every unseen underside.</p></details>
<p><a href="handoff.json">Frozen evidence and scope</a></p>''')
assert sum(p.stat().st_size for p in O.iterdir()) <= 32 * 1024**2
print(json.dumps(handoff, indent=2))
