"""Package the additive flower correction without changing authored appearance."""
import json
import sys
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT, scenery_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_inventory import inventory, validate_catalog
from refinement_workspace import prepare, validate, _render, _geometry
from audit_candidates import audit

ASSET = 'croisement02-shrub-76'


def main():
    source = OUT / 'understory-candidates/native76-foreground-v2'
    dest = OUT / 'restart2-fence/flower76-package-v1'
    dest.mkdir(exist_ok=False, parents=True)
    original = scenery_workspace(ASSET)
    protected = {str(p): sha(p) for p in [source / 'model.blend', original / 'model.blend']}
    cfg = json.loads((original / 'workspace.json').read_text())
    catalog = json.loads((original / 'reference/grouping.json').read_text())
    group = next(g for g in catalog['groups'] if g['id'] == ASSET)
    catalog['groups'] = [group]
    if catalog.get('version') == 2:
        catalog['canonical_owners'] = {'foliage-shrub-076': ASSET}
    write_json(dest / 'catalog.json', catalog)
    manifest = json.loads((original / 'source-masks.json').read_text())
    mask_path = Path(manifest['mask_inventory'])
    masks = json.loads(mask_path.read_text())
    for row in masks['masks']:
        row['png'] = str((mask_path.parent / row['png']).resolve())
    assert not any(row['index'] == 6002 for row in masks['masks'])
    boundary = OUT / 'mixed-wood-audit/boundary-roles76-93-v1/76-foliage76.png'
    masks['masks'].append(dict(index=6002, layer=0, png=str(boundary),
                              box_top_left=[0, 0], box_size=[1792, 1152],
                              provenance='Private separately inferred22 boundary leaf pixels; observed502 remains exact.'))
    write_json(dest / 'mask-inventory.json', masks)
    manifest['mask_inventory'] = str(dest / 'mask-inventory.json')
    for projection in manifest['projections'].values():
        projection['assignments'] = [r for r in projection['assignments']
                                     if r.get('source_node') == 'foliage-shrub-076']
        for row in projection['assignments']:
            assert row['mask_indices'] == [502]
            row['mask_indices'] = [502, 6002]
        projection['occluder_constraints'] = []
    write_json(dest / 'source-masks.json', manifest)
    bpy.ops.wm.open_mainfile(filepath=str(source / 'model.blend'))
    bpy.context.preferences.filepaths.save_version = 0
    for obj in list(bpy.data.objects):
        if obj.type == 'MESH' and obj.get('asset_group') != ASSET:
            bpy.data.objects.remove(obj, do_unlink=True)
    bpy.data.orphans_purge(do_recursive=True)
    collection = bpy.data.collections['Croisement02 Working']
    for obj in bpy.context.scene.objects:
        if obj.type == 'MESH' and obj.get('asset_group') == ASSET and obj.name not in collection.all_objects:
            collection.objects.link(obj)
    before = {o.name: _geometry(o, protect_appearance=True)
              for o in collection.all_objects if o.type == 'MESH'}
    assert len(before) == 6
    inventory(dest / 'inventory', collection_name=collection.name,
              map_name='Croisement02', source_path=cfg['source_path'])
    validate_catalog(dest / 'inventory/inventory.json', dest / 'catalog.json')
    write_json(dest / 'grouping-review.json', dict(
        status='reviewed', reviewer='Codex', catalog_sha256=sha(dest / 'catalog.json'),
        inventory_sha256=sha(dest / 'inventory/inventory.json'),
        evidence='Private scope audit only: one existing asset and unchanged source identity. '
                 'This is not geometry, boundary-role, or publication approval.'))
    scoped = dest / 'authored-scope.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(scoped), compress=True)
    worker = dest / 'assets' / ASSET
    prepare(worker, asset_id=ASSET, scene_name=cfg['scene_name'],
            collection_name=collection.name, source_path=cfg['source_path'],
            grouping_manifest=dest / 'catalog.json',
            inventory_path=dest / 'inventory/inventory.json',
            review_path=dest / 'grouping-review.json',
            source_mask_manifest=dest / 'source-masks.json',
            width=384, height=384, framing_padding=1.25, lighting=cfg['lighting'])
    bpy.ops.wm.open_mainfile(filepath=str(scoped))
    config = json.loads((worker / 'workspace.json').read_text())
    _render(config, worker / 'modified', worker / 'input/views.json')
    bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'), compress=True)
    bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
    after = {o.name: _geometry(o, protect_appearance=True)
             for o in bpy.data.collections['Croisement02 Working'].all_objects
             if o.type == 'MESH'}
    assert before == after, 'Packaging changed reviewed candidate appearance'
    assert all(sha(Path(p)) == digest for p, digest in protected.items())
    validate(worker)
    (worker / 'inspection').mkdir(exist_ok=True)
    audit(worker)
    write_json(worker / 'inspection/package-preservation.json', dict(
        status='PASS private package; independent review pending',
        model_sha256=sha(worker / 'model.blend'), protected=protected,
        exact_geometry_uv_material_preservation=True,
        source_candidate=str(source),
        limitations=['Canonical selection unchanged; source roles and geometry await root review.',
                     '162 additive fragments use inferred hidden depth beside the physical fence.',
                     'Observed502 and inferred6002 remain distinct source masks.',
                     'Pair evidence is in wattle99-source-candidate/v6/flower-joint-completed-v1.']))
    print(worker)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
