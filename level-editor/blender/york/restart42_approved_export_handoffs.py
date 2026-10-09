"""Bind approved York private exports and exact surface guards for root integration."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2'
CONFIG = {
    'well': ('restart42-approved-well-export-v2', 'restart38-well-texture-baked-v1'),
    'storehouse': ('restart42-approved-storehouse-export-v1', 'restart38-storehouse-texture-baked-v3'),
}
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
for key, (export_dir, source_dir) in CONFIG.items():
    out, source = BASE / export_dir, BASE / source_dir
    report = json.loads((out / 'export-report.json').read_text())
    guard = json.loads((out / 'source-surface-guard.json').read_text())
    model = out / '3d-assets' / report['asset_id'] / 'model.glb'
    descriptor = model.with_name('asset.json')
    live = ROOT / 'level-editor/library/3d-assets/york' / report['asset_id'] / 'asset.json'
    assert sha(live) == report['live_descriptor_sha256']
    assert sha(source / 'model.blend') == report['approved_model_sha256'] == guard['approved_model_sha256']
    assert sha(model) == report['model_sha256'] == guard['exported_glb_sha256']
    assert sha(descriptor) == report['descriptor_sha256']
    assert guard['status'].startswith('PASS ')
    files = [model, descriptor, out / '3d-assets/index.json', out / 'export-report.json',
             out / 'source-surface-guard.json', out / 'private-catalog.json',
             source / 'model.blend', source / 'grouped-review-handoff.json']
    handoff = dict(status='PRIVATE_APPROVED_EXPORT_SURFACE_GUARDED', asset_id=report['asset_id'],
                   files={str(p): sha(p) for p in files},
                   source_origin_scene=report['source_origin_scene'],
                   approved_model_sha256=report['approved_model_sha256'],
                   approval_receipt_sha256=report['approval_receipt_sha256'],
                   gameplay_matches_live_descriptor_sha256=report['live_descriptor_sha256'],
                   component_identity_mapping=report['source_components'],
                   source_component_display_labels=report['source_component_display_labels'],
                   triangles=sum(row['triangles'] for row in guard['objects']),
                   checks=['All approved parts and triangles exported exactly once with original winding.',
                           'Actual material UV channels correspond to the exact approved triangle corners.',
                           'All atlas RGBA bytes, opaque/unlit mode and sidedness preserved.',
                           'Approved source worker and live canonical descriptor remain unchanged.',
                           'Private descriptor keeps current pivot and complete gameplay record.'],
                   remaining=['Browser direct-model inspection queued with scatter_verification.',
                              'Root must use the supported production conversion/staging path and validate final map grouping and selection.',
                              'Canonical catalog must record reviewed component IDs where split source nodes require them.',
                              'No canonical writes, runtime verification or publication is claimed.'])
    target = out / 'integration-handoff.json'
    assert not target.exists(), target
    target.write_text(json.dumps(handoff, indent=2) + '\n')
    print(key, target, sha(target))
