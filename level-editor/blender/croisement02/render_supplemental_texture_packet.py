"""Reproduce source-only ownership using already reviewed full-crown cameras."""
import argparse
import copy
import json
from pathlib import Path
import sys
import bpy
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, tree_workspace
from review_evidence import sha
from render_slots import acquire, release
from refinement_review import render_review
from refinement_workspace import _geometry


def main(number, output):
    asset = f'croisement02-tree-{number:02d}'
    worker = tree_workspace(number)
    records = [r for r in json.loads((OUT / 'user-feedback.json').read_text())['records'] if r['asset_id'] == asset]
    decision = records[-1]
    if decision['decision'] != 'approved' or sha(worker / 'model.blend') != decision['model_sha256']:
        raise ValueError('Current geometry is not approved')
    archive = Path(decision['archive'])
    gallery = json.loads((archive / 'gallery-item.json').read_text())
    reviewed_audit = gallery['reports']['stored_material_full-crown_audit']
    audit_path = archive / reviewed_audit['file']
    if sha(audit_path) != reviewed_audit['sha256']:
        raise ValueError('Reviewed supplemental audit changed')
    audit = json.loads(audit_path.read_text())
    cameras = worker / 'inspection/full-crown/cameras.json'
    if sha(cameras) != audit['supplemental_cameras_sha256'] or audit['model_sha256'] != decision['model_sha256']:
        raise ValueError('Supplemental cameras or model differ from approved evidence')
    original = json.loads(cameras.read_text())
    frames = copy.deepcopy(original)
    # The supplemental actual renderer uses the matrix. Legacy Euler aliases
    # can still describe the narrower old framing, so do not select them.
    for view in frames['views']:
        view.pop('camera_location', None)
        view.pop('camera_rotation_euler', None)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(archive / 'model.blend'))
        scene = bpy.data.scenes[frames['scene_name']]
        before = {o.name: _geometry(o, protect_appearance=True) for o in scene.objects}
        rendered = render_review(output, scene_name=frames['scene_name'], collection_name=frames['collection_name'],
            asset_id=asset, source_path=frames['source_image'], frame_manifest=frames,
            projection_layers=frames['projection_layers'], lighting=frames['lighting'],
            source_mask_manifest=frames['source_mask_manifest'], render_object_names=frames['object_names'])
        if before != {o.name: _geometry(o, protect_appearance=True) for o in scene.objects}:
            raise ValueError('Source-only reproduction changed approved scene')
        errors = []
        for actual, expected in zip(rendered['views'], original['views']):
            matrix_error = float(np.abs(np.asarray(actual['camera_matrix_world']) - np.asarray(expected['camera_matrix_world'])).max())
            errors.append(matrix_error)
            if matrix_error > 1e-6 or actual['ortho_scale'] != float(np.float32(expected['ortho_scale'])):
                raise ValueError('Reproduced camera differs from reviewed matrix')
        receipt = dict(version=1, asset_id=asset, model_sha256=decision['model_sha256'], status='PASS',
            original_geometry_decision=decision, approved_camera_manifest=str(cameras),
            approved_camera_manifest_sha256=sha(cameras), approved_supplemental_audit=str(audit_path),
            approved_supplemental_audit_sha256=sha(audit_path), geometry_and_appearance_unchanged=True,
            maximum_camera_matrix_error=max(errors), camera_scale_precision='Blender float32, identical to the approved actual renderer', source_review='pending manual inspection',
            scope='New source-only/ownership rendering of approved unchanged geometry and approved supplemental camera matrices; these derived pixels were not previously user-reviewed.',
            artifacts={str(p.relative_to(output)): sha(p) for p in sorted(output.rglob('*')) if p.is_file()})
        (output / 'derivation.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print(json.dumps({'output': str(output), 'status': 'PASS', 'maximum_camera_matrix_error': max(errors)}), flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('tree', type=int)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.tree, args.output.resolve())
