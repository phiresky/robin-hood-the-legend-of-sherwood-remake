"""Package the exact reviewed density candidate against a fresh approved baseline."""
import json
from pathlib import Path
import shutil
import sys

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT
from approved_texture_stage import geometry, appearance, require
from evidence_io import sha, write_json, record_recipe
from render_slots import acquire, release
from refinement_workspace import prepare, modified, validate
from audit_candidates import audit
from render_tree import render_workspace
from render_tree_prototype_comparison import main as compare


def main():
    trial = OUT / 'restart2-tree40/density-v5'
    previous = OUT / 'forest-v4-round-1/assets/croisement02-tree-40'
    output = OUT / 'restart2-tree40/packaged-v1/assets/croisement02-tree-40'
    require(not output.exists(), 'Use a fresh standard workspace')
    proof = json.loads((trial / 'root-review.json').read_text())
    model_hash = sha(trial / 'worker.blend')
    require(proof['model_sha256'] == model_hash and proof['status'] == 'ready-for-user-new-geometry-review',
            'Exact candidate has no root geometry review')
    previous_hash = sha(previous / 'model.blend')
    require(previous_hash == '24a07f56159531bfd7c0e20fd077cfbe7232e0aaad9f7bd1382820d3e61b66e0',
            'Previously approved tree40 changed')
    report = json.loads((previous / 'inspection/refinement.json').read_text())
    packet = Path(report['source_packet']).parent
    cfg = json.loads((previous / 'workspace.json').read_text())
    acquire()
    try:
        def wood_state():
            return {o.name: dict(geometry=geometry(o), appearance=appearance(o))
                    for o in bpy.data.collections[cfg['collection_name']].all_objects
                    if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id']
                    and o.get('projection_component') != 'crown'}
        bpy.ops.wm.open_mainfile(filepath=str(previous / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        old_wood = wood_state()
        prepare(output, asset_id=cfg['asset_id'], scene_name=cfg['scene_name'],
                collection_name=cfg['collection_name'], source_path=previous / 'reference/source.png',
                grouping_manifest=previous / 'reference/grouping.json',
                inventory_path=previous / 'reference/inventory.json',
                review_path=previous / 'reference/grouping-review.json',
                source_mask_manifest=previous / 'source-masks.json', width=384, height=384,
                framing_padding=1.25, lighting=cfg['lighting'])
        inspection = output / 'inspection'
        inspection.mkdir(exist_ok=True)
        shutil.copytree(packet, inspection / 'source-packet')
        shutil.copy2(trial / 'worker.blend', output / 'model.blend')
        bpy.ops.wm.open_mainfile(filepath=str(output / 'model.blend'))
        require(wood_state() == old_wood, 'Candidate changed approved wood')
        report.update(model_sha256=model_hash,
                      source_packet=str(inspection / 'source-packet/partition.json'),
                      status='New inferred canopy density; new geometry approval pending')
        report['crown'].update(geometry_version='inferred-boundary-density-v5',
            method='Thinned inferred outer crossed clusters and ragged leaf outlines; exact native observed crown retained',
            density_evidence_sha256=sha(trial / 'validation.json'))
        report['limitations'] = [
            'Physical inferred alpha changed, so old geometry approval does not apply.',
            'A denser/color transition remains near the original map boundary; native and inferred sides are asymmetric.',
            'Native pixels are exact; inferred RGB uses only the exact individual 9703-pixel tree40 donor.']
        write_json(inspection / 'refinement.json', report)
        modified(output)
        # Standard source-only review projection is separate from the exact
        # physically reviewed candidate materials.
        shutil.copy2(trial / 'worker.blend', output / 'model.blend')
        bpy.ops.wm.open_mainfile(filepath=str(output / 'model.blend'))
        write_json(output / 'validation.json', validate(output))
        require(wood_state() == old_wood, 'Packaging changed approved wood')
        write_json(inspection / 'prototype-preservation.json', dict(
            previous_worker=str(previous), previous_model_sha256=previous_hash,
            previous_model_unchanged=True, model_sha256=model_hash,
            non_crown_geometry_and_materials_preserved=True, preserved_non_crown=old_wood,
            trial_worker=str(trial), trial_model_sha256=model_hash,
            approval='pending', texture_generation='No API call for this geometry; scoped own-native synthesis retained',
            root_completion_base=None))
        for name in ['root-review.json', 'validation.json', 'visible-bounds.json', 'independent-visual-review.json']:
            shutil.copy2(trial / name, inspection / ('density-' + name))
        audit(output)
        record_recipe(output, __file__)
        record_recipe(output, Path(__file__).with_name('revise_tree40_density.py'))
        require(sha(output / 'model.blend') == model_hash, 'Exact reviewed model changed')
        require(sha(previous / 'model.blend') == previous_hash, 'Old approved model changed')
    finally:
        release()
    render_workspace(output, 384, transparent_bounces=256)
    compare(output)
    require(sha(output / 'model.blend') == model_hash, 'Rendering changed exact candidate')
    print(output, flush=True)


if __name__ == '__main__':
    main()
