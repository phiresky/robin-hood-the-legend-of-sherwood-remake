"""Remove the independently switched obstacle from the visible tree worker."""
import json
import sys
from pathlib import Path
import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import prepare, validate, modified
from bark_materials import fill
from audit_candidates import audit
from render_tree import render_workspace


def main():
    asset = 'croisement02-tree-03'
    old = OUT / 'forest-v4-round-1/assets' / asset
    worker = OUT / 'forest-v4-round-2/assets' / asset
    domain = OUT / 'tree03-state-revision'
    receipt = worker / 'inspection/source-domain-revision.json'
    latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    if latest.get(asset, {}).get('decision') == 'approved':
        raise ValueError('Approved geometry is frozen')
    states = json.loads((OUT / 'state-review/inventory.json').read_text())
    patch = next(p for p in states['native_patches'] if p['old_obstacles'] == [144])
    assert patch['new_obstacles'] == [] and patch['old_masks'] == [137]
    acquire()
    try:
        if not (worker / 'workspace.json').exists():
            domain.mkdir(exist_ok=True)
            catalog = json.loads((OUT / 'catalog.json').read_text())
            tree = next(g for g in catalog['groups'] if g['id'] == asset)
            tree['parts'] = [p for p in tree['parts'] if p['obstacle'] != 144]
            state_id = 'croisement02-west-covered-state'
            catalog['groups'].append(dict(id=state_id, name='West Covered State',
                parts=[dict(obstacle=144, name='West covered obstacle 144')],
                state_only=True, patch=patch['id']))
            write_json(domain / 'catalog.json', catalog)
            inventory = OUT / 'forest-v4-inventory/inventory.json'
            write_json(domain / 'grouping-review.json', dict(status='reviewed', reviewer='Codex',
                catalog_sha256=sha(domain / 'catalog.json'), inventory_sha256=sha(inventory),
                evidence='Native patch 006 independently disables obstacle 144 and mask 137. '
                         'The obstacle is state metadata, not a branch of tree 03. '
                         'All 150 native parts retain exactly one owner.'))
            bpy.ops.wm.open_mainfile(filepath=str(old / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            state = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                     if o.get('source_node') == 'building-144']
            if len(state) != 1:
                raise ValueError('Expected one state obstacle')
            state[0]['asset_group'] = state_id
            state[0]['asset_name'] = 'West Covered State'
            state[0]['state_only'] = True
            state[0]['reveal_hide_when_applied'] = patch['id']
            state[0].hide_render = True
            prepare(worker, asset_id=asset, scene_name='Croisement02 Refinement',
                collection_name='Croisement02 Working',
                source_path=OUT / 'animation-references/composite-frame-0.png',
                grouping_manifest=domain / 'catalog.json', inventory_path=inventory,
                review_path=domain / 'grouping-review.json',
                source_mask_manifest=old / 'source-masks.json', width=256, height=256,
                framing_padding=1.35,
                lighting=dict(toward_sun=[-.45, -.55, .70], ambient=.22, diffuse=.78, shadow_epsilon=.05))
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            validate(worker)
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == asset]
            assert {o['source_node'] for o in objects} == {'building-050', 'building-051'}
            modified(worker)
            bark = fill(worker, objects, 3, receiver_only=True, donor_mapping='aperiodic-vertical')
            validate(worker)
            bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
            report = json.loads((old / 'inspection/refinement.json').read_text())
            report.update(model_sha256=sha(worker / 'model.blend'), bark=bark,
                          wood=[r for r in report['wood'] if r['source_node'] != 'building-144'])
            report['limitations'].append('Patch-controlled obstacle 144 is separately owned state metadata; it is excluded from visible tree geometry.')
            write_json(worker / 'inspection/refinement.json', report)
            audit(worker)
            write_json(receipt, dict(model_sha256=report['model_sha256'],
                previous_model_sha256=sha(old / 'model.blend'), excluded_obstacle=144,
                native_patch=patch['id'], state_inventory_sha256=sha(OUT / 'state-review/inventory.json')))
        elif json.loads(receipt.read_text())['model_sha256'] != sha(worker / 'model.blend'):
            raise ValueError('Revised tree changed')
        render_workspace(worker, 256, release_slot=False)
    finally:
        release()


if __name__ == '__main__':
    main()
