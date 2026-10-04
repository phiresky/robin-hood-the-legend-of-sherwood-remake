"""Prepare York geometry workers without modifying the grouping milestone.

Run in background Blender, followed by ``-- <asset-id> ...``. Initial packets
are diagnostic baselines; they are not refined geometry or texture approvals.
"""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
PASS = OUT / 'geometry-pass-01'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire, release
    acquire()
    sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
    from freeze_tooling import select_tooling
    pointer = json.loads((OUT / 'tooling/current.json').read_text())
    tooling = select_tooling(pointer['directory'])
    import bpy
    from refinement_inventory import validate_catalog
    from refinement_workspace import prepare
    from catalog_schema import parse_catalog
    catalog = ROOT / 'level-editor/refinement/catalogs/york.json'
    inventory = OUT / 'inventory/inventory.json'
    source = OUT / 'grounding/york-grounded.blend'
    assets = sys.argv[sys.argv.index('--') + 1:]
    if not assets:
        raise ValueError('Specify explicit asset IDs')
    validation = validate_catalog(inventory, catalog)
    index = parse_catalog(json.loads(catalog.read_text()))
    bpy.ops.wm.open_mainfile(filepath=str(source))
    objects = [o for o in bpy.data.collections['york Working'].all_objects
               if o.type == 'MESH' and o.get('source_node') != 'ground']
    index.validate_meshes([{'source_node': o.get('source_node'),
                           'projection_component': o.get('projection_component'),
                           'hide_render': o.hide_render} for o in objects])
    for obj in objects:
        if obj.hide_render:
            continue
        owner, _ = index.owner_for(obj['source_node'], obj.get('projection_component'))
        if owner['id'] != obj.get('asset_group'):
            raise ValueError('Grounded scene ownership differs: ' + obj.name)
    PASS.mkdir(exist_ok=True)
    review = PASS / 'grouping-reconciliation.json'
    record = {'status': 'reviewed', 'reviewer': 'Codex',
              'catalog_sha256': sha(catalog), 'inventory_sha256': sha(inventory),
              'source_blend_sha256': sha(source),
              'scope': 'Existing grounded scene component ownership reconciled with current catalog. No geometry or texture approval.',
              'previous_grouping_review_sha256': sha(OUT / 'grouping-review.json'),
              'validation': validation, 'tooling': tooling['snapshot_id']}
    review.write_text(json.dumps(record, indent=2) + '\n')
    for asset in assets:
        bpy.ops.wm.open_mainfile(filepath=str(source))
        bpy.context.preferences.filepaths.save_version = 0
        prepare(PASS / 'assets' / asset, asset_id=asset,
                scene_name='york Refinement', collection_name='york Working',
                source_path=OUT / 'baseline/covered.png', grouping_manifest=catalog,
                inventory_path=inventory, review_path=review)
        workspace = PASS / 'assets' / asset
        (workspace / 'diagnostic-scope.json').write_text(json.dumps({
            'status': 'baseline-only', 'ready_for_approval': False,
            'limitations': ['Source-mask assignments require per-asset review.',
                           'Revealed states are not yet reviewed.',
                           'Default diagnostic lighting is not York artwork calibration.'],
            'model_sha256': sha(workspace / 'model.blend')}, indent=2) + '\n')
        release()
        if asset != assets[-1]:
            acquire()


if __name__ == '__main__':
    main()
