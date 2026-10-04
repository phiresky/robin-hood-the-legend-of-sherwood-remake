"""Prepare a new review worker for the privately inspected continuous stem."""
import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import prepare, modified, validate
from audit_candidates import audit
from render_tree import render_workspace
from revise_feedback import geometry
from tree_geometry import SIN, COS


def main(candidate, destination, resume=False):
    asset = 'croisement02-supplemental-wood-44'
    old = OUT / 'authored-stems-round-1/assets' / asset
    worker = destination / 'assets' / asset
    proof = json.loads((candidate / 'join-experiment.json').read_text())
    old_hash, candidate_hash = sha(old / 'model.blend'), sha(candidate / 'model.blend')
    assert proof['approved_model_sha256'] == old_hash and proof['model_sha256'] == candidate_hash
    assert proof['source_coverage']['intersection_over_union'] >= .95
    assert not worker.exists() or (resume and not (worker / 'inspection/visual-review.json').exists())
    cfg = json.loads((old / 'workspace.json').read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(old / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        if not worker.exists():
            prepare(worker, asset_id=asset, scene_name=cfg['scene_name'], collection_name=cfg['collection_name'],
                source_path=old / 'reference/source.png', grouping_manifest=old / 'reference/grouping.json',
                inventory_path=old / 'reference/inventory.json', review_path=old / 'reference/grouping-review.json',
                source_mask_manifest=old / 'source-masks.json', width=256, height=384,
                framing_padding=1.25, lighting=cfg['lighting'])
        bpy.ops.wm.open_mainfile(filepath=str(candidate / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        objects = [o for o in bpy.data.collections[cfg['collection_name']].all_objects
                   if o.type == 'MESH' and o.get('asset_group') == asset]
        signature = geometry(objects)
        for obj in objects:
            if not obj.data.uv_layers:
                obj.data.uv_layers.new(name='Neutral fallback before native source projection')
        bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
        modified(worker)
        assert geometry(objects) == signature, 'Native reprojection changed candidate geometry'
        inspection = worker / 'inspection'
        inspection.mkdir(exist_ok=True)
        report = json.loads((old / 'inspection/refinement.json').read_text())
        report.update(model_sha256=sha(worker / 'model.blend'), status='New joined stem geometry; user approval pending')
        report['limitations'] = ['Continuous joined fork replaces three separately capped raster-row sweeps; former collar rings removed.',
            'Small side undulations remain. Main trunk depth is approximately its local width; exact diagnostics are retained.',
            'Bottom stem extends thirty pixels beyond the map; that continuation and unseen depth are inferred.',
            'Native known bark reprojected with existing exclusions; unseen bark remains neutral until new geometry approval.',
            'Earlier geometry and texture decisions do not apply to this changed stem. Original worker and fill preserved.']
        write_json(inspection / 'refinement.json', report)
        write_json(inspection / 'prototype-preservation.json', dict(previous_worker=str(old), previous_model_sha256=old_hash,
            previous_model_unchanged=True, model_sha256=sha(worker / 'model.blend'),
            private_candidate=str(candidate), private_candidate_sha256=candidate_hash,
            geometry_matches_private_candidate=True, approval='pending', texture_generation='not performed'))
        points = np.array([o.matrix_world @ v.co for o in objects for v in o.data.vertices])
        projected_y = -points[:, 1] * SIN - points[:, 2] * COS
        rows = []
        for y in [1090, 1100, 1110, 1120, 1130, 1140, 1150, 1160, 1170]:
            section = points[np.abs(projected_y - y) < .6]
            width, depth = float(np.ptp(section[:, 0])), float(np.ptp(section[:, 1]))
            rows.append(dict(source_y=y, width=width, depth=depth, depth_width_ratio=depth / width))
        write_json(inspection / 'local-depth.json', dict(model_sha256=sha(worker / 'model.blend'), rows=rows,
            method='World X width and world Y depth within a 1.2-pixel native source row slab.',
            interpretation='Rows1120–1150 sample the main trunk (roughly round). Earlier rows include fork transition width; later rows sample the inferred rounded continuation. No global depth inflation performed.'))
        audit(worker)
        render_workspace(worker, 256, release_slot=False)
        coverage = json.loads((inspection / 'source-coverage/report.json').read_text())
        assert coverage['intersection_over_union'] >= .95
        assert sha(old / 'model.blend') == old_hash
        from render_stem_comparison import main as compare
        compare(worker)
        print(worker, flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.candidate.resolve(), args.destination.resolve(), args.resume)
