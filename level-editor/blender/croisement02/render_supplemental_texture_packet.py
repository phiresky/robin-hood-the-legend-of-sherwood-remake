"""Render texture inputs from approved cameras without changing framing or geometry."""
import argparse
import copy
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from mathutils import Matrix
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, tree_workspace
from review_evidence import sha
from render_slots import acquire, release
from refinement_review import render_review
from refinement_workspace import _geometry


def main(number, output, camera_source="supplemental", resolution_factor=1, fit_complete=False):
    asset = f'croisement02-tree-{number:02d}'
    worker = tree_workspace(number)
    records = [r for r in json.loads((OUT / 'user-feedback.json').read_text())['records'] if r['asset_id'] == asset]
    decision = records[-1]
    if decision['decision'] != 'approved' or sha(worker / 'model.blend') != decision['model_sha256']:
        raise ValueError('Current geometry is not approved')
    archive = Path(decision['archive'])
    gallery = json.loads((archive / 'gallery-item.json').read_text())
    if camera_source == 'supplemental':
        reviewed_audit = gallery['reports']['stored_material_full-crown_audit']
        audit_path = archive / reviewed_audit['file']
        if sha(audit_path) != reviewed_audit['sha256']:
            raise ValueError('Reviewed supplemental audit changed')
        audit = json.loads(audit_path.read_text())
        cameras = worker / 'inspection/full-crown/cameras.json'
        if sha(cameras) != audit['supplemental_cameras_sha256'] or audit['model_sha256'] != decision['model_sha256']:
            raise ValueError('Supplemental cameras or model differ from approved evidence')
        camera_binding = dict(approved_supplemental_audit=str(audit_path), approved_supplemental_audit_sha256=sha(audit_path))
    else:
        cameras = archive / 'modified/views.json'
        if sha(cameras) != sha(worker / 'modified/views.json'):
            raise ValueError('Original reviewed camera archive differs from current approved worker')
        for name in ('solid', 'textured'):
            if sha(archive / f'modified/{name}.png') != decision[name + '_sha256']:
                raise ValueError('Original approved review image changed')
        camera_binding = dict(approved_original_archive=str(archive), approved_original_decision_sha256=sha(archive / 'decision.json'))
    original = json.loads(cameras.read_text())
    frames = copy.deepcopy(original)
    if type(resolution_factor) is not int or resolution_factor not in (1, 2, 3):
        raise ValueError('Resolution factor must be one of 1, 2, or 3')
    frames['tile_size'] = [value * resolution_factor for value in original['tile_size']]
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
        if fit_complete:
            points = [obj.matrix_world @ vertex.co for obj in scene.objects
                      if obj.name in frames['object_names'] and obj.type == 'MESH'
                      for vertex in obj.data.vertices]
            old_size = (scene.render.resolution_x, scene.render.resolution_y)
            scene.render.resolution_x, scene.render.resolution_y = frames['tile_size']
            camera = bpy.data.cameras.new('Temporary framing measurement')
            camera.type = 'ORTHO'
            try:
                for view in frames['views']:
                    camera.ortho_scale = view['ortho_scale']
                    boundary = camera.view_frame(scene=scene)
                    left, right = min(p.x for p in boundary), max(p.x for p in boundary)
                    bottom, top = min(p.y for p in boundary), max(p.y for p in boundary)
                    inverse = Matrix(view['camera_matrix_world']).inverted()
                    local = [inverse @ point for point in points]
                    needed = max(max(p.x / right, p.x / left, p.y / top, p.y / bottom) for p in local)
                    view['ortho_scale'] *= max(1.15, needed * 1.10)
            finally:
                bpy.data.cameras.remove(camera)
                scene.render.resolution_x, scene.render.resolution_y = old_size
        rendered = render_review(output, scene_name=frames['scene_name'], collection_name=frames['collection_name'],
            asset_id=asset, source_path=frames['source_image'], frame_manifest=frames,
            projection_layers=frames['projection_layers'], lighting=frames['lighting'],
            source_mask_manifest=frames['source_mask_manifest'], render_object_names=frames['object_names'])
        if before != {o.name: _geometry(o, protect_appearance=True) for o in scene.objects}:
            raise ValueError('Source-only reproduction changed approved scene')
        errors = []
        for actual, expected in zip(rendered['views'], frames['views']):
            matrix_error = float(np.abs(np.asarray(actual['camera_matrix_world']) - np.asarray(expected['camera_matrix_world'])).max())
            errors.append(matrix_error)
            if matrix_error > 1e-6 or actual['ortho_scale'] != float(np.float32(expected['ortho_scale'])):
                raise ValueError('Reproduced camera differs from reviewed matrix')
        framing_audit = []
        for view in rendered['views']:
            alpha = np.asarray(Image.open(output / 'views' / f"view-{view['index']}-solid.png").convert('RGBA'))[:, :, 3] > 0
            rows, cols = np.where(alpha)
            touches = bool(alpha[0].any() or alpha[-1].any() or alpha[:, 0].any() or alpha[:, -1].any())
            framing_audit.append(dict(view=view['index'], touches_boundary=touches,
                bounds=[int(cols.min()), int(rows.min()), int(cols.max() + 1), int(rows.max() + 1)]))
        if fit_complete and any(view['touches_boundary'] for view in framing_audit):
            raise ValueError('Complete framing still clips a rendered silhouette')
        receipt = dict(version=1, asset_id=asset, model_sha256=decision['model_sha256'], status='PASS',
            original_geometry_decision=decision, approved_camera_manifest=str(cameras),
            approved_camera_manifest_sha256=sha(cameras), camera_source=camera_source,
            original_tile_size=original['tile_size'], rendered_tile_size=frames['tile_size'],
            resolution_factor=resolution_factor, image_resampling=False, **camera_binding, geometry_and_appearance_unchanged=True,
            framing_adjustment='ortho-scale-only to include the complete mesh with padding' if fit_complete else None,
            original_ortho_scales=[v['ortho_scale'] for v in original['views']],
            rendered_ortho_scales=[v['ortho_scale'] for v in rendered['views']], framing_audit=framing_audit,
            maximum_camera_matrix_error=max(errors), camera_scale_precision='Blender float32, the same storage precision used by the approved actual renderer', source_review='pending manual inspection',
            scope='New source-only/ownership rendering of approved unchanged geometry and approved camera orientations/positions with explicitly recorded framing and render resolution; these derived pixels were not previously user-reviewed.',
            artifacts={str(p.relative_to(output)): sha(p) for p in sorted(output.rglob('*')) if p.is_file()})
        (output / 'derivation.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print(json.dumps({'output': str(output), 'status': 'PASS', 'maximum_camera_matrix_error': max(errors)}), flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('tree', type=int)
    parser.add_argument('output', type=Path)
    parser.add_argument('--camera-source', choices=('supplemental', 'original'), default='supplemental')
    parser.add_argument('--resolution-factor', type=int, default=1, help='New render resolution; never resamples an image')
    parser.add_argument('--fit-complete', action='store_true', help='Explicit new preparation framing: retain camera transforms and enlarge scales to include all geometry')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.tree, args.output.resolve(), args.camera_source, args.resolution_factor, args.fit_complete)
