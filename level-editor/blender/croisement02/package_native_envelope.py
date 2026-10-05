"""Package a private crown trial against its unchanged approved baseline."""
import argparse
import json
from pathlib import Path
import shutil
import sys

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
                str(ROOT / 'level-editor/refinement/blender')]
from approved_texture_stage import geometry, appearance, require
from evidence_io import sha, write_json, record_recipe
from render_slots import acquire, release
from refinement_workspace import modified, validate
from audit_candidates import audit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trial', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    trial, output = args.trial.resolve(), args.output.resolve()
    require(not output.exists(), 'Use a fresh packaged workspace')
    trial_proof = json.loads((trial / 'inspection/envelope-preservation.json').read_text())
    source = Path(trial_proof['source_worker'])
    require(sha(trial / 'model.blend') == trial_proof['model_sha256'], 'Trial changed')
    require(sha(source / 'model.blend') == trial_proof['source_model_sha256'], 'Construction source changed')
    old_proof_path = source / 'inspection/prototype-preservation.json'
    if old_proof_path.exists():
        old_proof = json.loads(old_proof_path.read_text())
        previous = Path(old_proof['previous_worker'])
        previous_hash = old_proof['previous_model_sha256']
    else:
        # An interrupted private trial may lack its final report. Its frozen
        # workspace still binds the exact source; actual wood is checked below.
        original_config = json.loads((source / 'workspace.json').read_text())
        previous = Path(original_config['source_blend']).parent
        previous_hash = original_config['source_blend_sha256']
        from catalog import OUT
        feedback = json.loads((OUT / 'user-feedback.json').read_text())['records']
        require(any(r['asset_id'] == original_config['asset_id'] and r.get('decision') == 'approved'
                    and r.get('model_sha256') == previous_hash for r in feedback),
                'Interrupted trial has no exact approved baseline')
    require(sha(previous / 'model.blend') == previous_hash, 'Approved baseline changed')
    output.mkdir(parents=True)
    for name in ['input', 'reference', 'mask-reference']:
        shutil.copytree(source / name, output / name)
    for name in ['baseline.blend', 'source-masks.json', 'workspace.json']:
        shutil.copy2(source / name, output / name)
    shutil.copy2(trial / 'model.blend', output / 'model.blend')
    shutil.copytree(trial / 'inspection', output / 'inspection')
    shutil.copytree(source / 'inspection/source-packet', output / 'inspection/source-packet')
    config = json.loads((output / 'workspace.json').read_text())
    config.update(mask_reference=str(output / 'mask-reference'),
                  source_mask_manifest=str(output / 'source-masks.json'))
    # Frozen review cameras bind the original source path as well as its hash.
    # Keep that read-only reference while also shipping its identical local copy.
    write_json(output / 'workspace.json', config)
    acquire()
    try:
        def wood_state():
            return {o.name: dict(geometry=geometry(o), appearance=appearance(o))
                    for o in bpy.data.collections[config['collection_name']].all_objects
                    if o.type == 'MESH' and o.get('asset_group') == config['asset_id']
                    and o.get('projection_component') != 'crown'}
        bpy.ops.wm.open_mainfile(filepath=str(previous / 'model.blend'))
        old_wood = wood_state()
        bpy.ops.wm.open_mainfile(filepath=str(output / 'model.blend'))
        require(wood_state() == old_wood, 'Trial changed previously approved wood')
        bpy.context.preferences.filepaths.save_version = 0
        modified(output)
        # The standard packet exposes known surfaces. Keep the exact candidate
        # appearance that was rendered and reviewed independently.
        shutil.copy2(trial / 'model.blend', output / 'model.blend')
        bpy.ops.wm.open_mainfile(filepath=str(output / 'model.blend'))
        validation = validate(output)
        write_json(output / 'validation.json', validation)
        require(wood_state() == old_wood, 'Packaging changed approved wood')
        report = json.loads((output / 'inspection/refinement.json').read_text())
        crown, = [o for o in bpy.data.collections[config['collection_name']].all_objects
                  if o.type == 'MESH' and o.get('asset_group') == config['asset_id']
                  and o.get('projection_component') == 'crown']
        report.update(model_sha256=sha(output / 'model.blend'),
            source_packet=str(output / 'inspection/source-packet/partition.json'),
            status='New native leaf envelope geometry; approval pending')
        report['crown'].update(geometry_version='native-fragment-envelope-v1',
            vertices=len(crown.data.vertices), faces=len(crown.data.polygons),
            method=trial_proof['method'], construction_evidence_sha256=sha(trial / 'inspection/envelope-preservation.json'))
        write_json(output / 'inspection/refinement.json', report)
        write_json(output / 'inspection/prototype-preservation.json', dict(
            previous_worker=str(previous), previous_model_sha256=sha(previous / 'model.blend'),
            previous_model_unchanged=True, model_sha256=sha(output / 'model.blend'),
            non_crown_geometry_and_materials_preserved=True, preserved_non_crown=old_wood,
            trial_worker=str(trial), trial_model_sha256=sha(trial / 'model.blend'),
            approval='pending', texture_generation='not performed', root_completion_base=None))
        audit(output)
        record_recipe(output, __file__)
        record_recipe(output, Path(__file__).with_name('smooth_native_crown_candidate.py'))
        for recipe, expected in trial_proof.get('dependency_recipes', {}).items():
            require(sha(Path(recipe)) == expected, 'Trial dependency recipe changed')
            record_recipe(output, Path(recipe))
        require(sha(output / 'model.blend') == sha(trial / 'model.blend'), 'Exact reviewed trial changed')
        print('PASS: exact trial packaged against approved baseline', flush=True)
    finally:
        release()


if __name__ == '__main__':
    main()
