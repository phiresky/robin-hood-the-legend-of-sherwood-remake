"""Replace inferred circular crown cuts without changing approved trees.

Native canopy alpha and source RGB remain authoritative. Individual tree edges
inside a shared canopy are inferred, irregular overlaps, not recovered borders.
"""
import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from forest_layout import CROWNS
from tree_geometry import crown_geometry
from bark_materials import fill
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import validate, modified
from audit_candidates import audit
from render_tree import render_workspace


def revise(row):
    workspace = OUT / 'forest-v4-round-1/assets' / row['asset_id']
    report_path = workspace / 'inspection/refinement.json'
    record = json.loads(report_path.read_text())
    destination = workspace / 'inspection/irregular-crown-edge'
    receipt = destination / 'revision.json'
    if receipt.exists():
        saved = json.loads(receipt.read_text())
        if saved['model_sha256'] != sha(workspace / 'model.blend'):
            raise ValueError('Revised crown changed; inspect before resuming')
    else:
        if record.get('source_packet'):
            raise ValueError('Already has a separate source revision: ' + workspace.name)
        packet_path = Path(row['packet'])
        packet = json.loads(packet_path.read_text())
        x, y, width, height = packet['native_bbox']
        rgba = np.asarray(Image.open(packet_path.parent / 'complete-source.png')).copy()
        previous = rgba[:, :, 3] > 127
        native = json.loads((OUT / 'baseline/masks/manifest.json').read_text())
        native_row = next(r for r in native['masks'] if r['index'] == packet['native_mask'])
        native_alpha = np.asarray(Image.open(OUT / 'baseline/masks' / native_row['png']).convert('L')) > 0
        _, cx, cy = next(c for c in CROWNS[row['animation']] if c[0] == row['mask'])
        yy, xx = np.mgrid[:height, :width]
        dx, dy = xx + x - cx, yy + y - cy
        distance, theta = np.hypot(dx, dy), np.arctan2(dy, dx)
        radius = float(distance[previous].max())
        phase = row['mask'] * 2.399963229728653
        # A small overlap remains; varied lobes replace the exact circular arc.
        edge = radius + 9 + 9*np.sin(11*theta+phase) + 5*np.sin(29*theta-phase) + 3*np.sin(67*theta)
        alpha = native_alpha & (distance <= edge)
        if not alpha.any():
            raise ValueError('Empty revised crown')
        rgba[:, :, 3] = alpha * 255
        destination.mkdir(exist_ok=True)
        Image.fromarray(rgba).save(destination / 'complete-source.png')
        Image.fromarray((alpha*255).astype('uint8')).save(destination / 'coverage.png')
        ay, ax = np.nonzero(alpha)
        packet['bbox'] = [x+int(ax.min()), y+int(ay.min()), int(np.ptp(ax))+1, int(np.ptp(ay))+1]
        packet['source_pixels'] = int(alpha.sum())
        packet['lobes'] = [dict(image=str(destination / 'complete-source.png'))]
        packet['coverage_provenance'] = 'Native canopy alpha with an irregular inferred per-tree overlap. Shared artwork does not identify an exact individual crown boundary.'
        write_json(destination / 'partition.json', packet)
        write_json(destination / 'previous-refinement.json', record)
        before_hash = sha(workspace / 'model.blend')
        cfg = json.loads((workspace / 'workspace.json').read_text())
        masks_path = Path(cfg['source_mask_manifest'])
        masks = json.loads(masks_path.read_text())
        write_json(destination / 'previous-source-masks.json', masks)
        assignment = next(a for a in masks['projections']['exterior']['assignments']
                          if a.get('source_node') == f"building-{row['primary']:03}"
                          and a.get('projection_component') == 'crown')
        assignment['mask_indices'] = [packet['native_mask']]
        assignment['review_note'] = packet['coverage_provenance']
        acquire()
        bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        validate(workspace)
        write_json(masks_path, masks)
        objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                   if o.type == 'MESH' and o.get('asset_group') == workspace.name]
        crown = next(o for o in objects if o.get('projection_component') == 'crown')
        record['crown'] = crown_geometry(crown, packet, row['ground_y'], True)
        crown['foliage_backfaces_version'] = 'v5'
        modified(workspace)
        record['bark'] = fill(workspace, objects, row['mask'])
        validate(workspace)
        bpy.ops.wm.save_as_mainfile(filepath=str(workspace / 'model.blend'))
        record['source_packet'] = str(destination / 'partition.json')
        record['model_sha256'] = sha(workspace / 'model.blend')
        record['leaf_fallback_ownership'] = 'corrected'
        record['limitations'] = [n for n in record['limitations'] if not n.startswith('Rounded overlapping supports')]
        record['limitations'].append(packet['coverage_provenance'])
        write_json(report_path, record)
        audit(workspace)
        write_json(receipt, dict(before_model_sha256=before_hash, model_sha256=record['model_sha256'],
                                source_rgb_unchanged=True, native_mask=packet['native_mask'],
                                added_pixels=int((alpha & ~previous).sum()), removed_pixels=int((previous & ~alpha).sum())))
    evidence = workspace / 'inspection/actual-materials/evidence.json'
    coverage = workspace / 'inspection/source-coverage/report.json'
    current = sha(workspace / 'model.blend')
    if not all(p.exists() and json.loads(p.read_text()).get('model_sha256') == current for p in (evidence, coverage)):
        render_workspace(workspace, 256, release_slot=False)
    release()
    print('REVISED', workspace.name, flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--masks', nargs='+', type=int, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    decisions = json.loads((OUT / 'user-feedback.json').read_text())['records']
    latest = {r['asset_id']: r for r in decisions}
    rows = json.loads((OUT / 'forest-v4-sources/manifest.json').read_text())
    selected = [r for r in rows if r['mask'] in args.masks]
    if {r['mask'] for r in selected} != set(args.masks):
        raise ValueError('Unknown requested wood mask')
    for row in selected:
        if latest.get(row['asset_id'], {}).get('decision') == 'approved':
            raise ValueError('Refusing to revise approved geometry: ' + row['asset_id'])
    try:
        for row in selected:
            revise(row)
    finally:
        release()


if __name__ == '__main__':
    main()
