"""Complete clipped crowns outside the map using their own leaf patches."""
import argparse
import json
import sys
import uuid
from pathlib import Path
import bpy
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT
from tree_geometry import replace_mesh, RAY, SIN, COS
from evidence_io import sha, write_json
from refinement_workspace import validate, modified
from render_slots import acquire, release
from audit_candidates import audit
from render_tree import render_workspace
from bark_materials import fill
from rounded_boundary_geometry import build


def complete(mask):
    worker = OUT / f'forest-v4-round-1/assets/croisement02-tree-{mask:02}'
    receipt = worker / 'inspection/boundary-completion.json'
    latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
    if latest.get(worker.name, {}).get('decision') == 'approved':
        raise ValueError('Approved geometry is frozen')
    acquire()
    try:
        if receipt.exists() and '--redo' in sys.argv:
            receipt.rename(receipt.with_name('boundary-completion-archive-' + uuid.uuid4().hex[:8] + '.json'))
        if not receipt.exists():
            bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
            bpy.context.preferences.filepaths.save_version = 0
            validate(worker)
            objects = [o for o in bpy.data.collections['Croisement02 Working'].all_objects
                       if o.type == 'MESH' and o.get('asset_group') == worker.name]
            crown = next(o for o in objects if o.get('projection_component') == 'crown')
            report = json.loads((worker / 'inspection/refinement.json').read_text())
            packet = json.loads(Path(report['source_packet']).read_text())
            row = next(r for r in json.loads((OUT / 'forest-v4-sources/manifest.json').read_text()) if r['mask'] == mask)
            boundary = 0 if mask == 24 else 1792
            before = sha(worker / 'model.blend')
            result = build(crown, packet, row['ground_y'], boundary)
            modified(worker)
            bark = fill(worker, objects, mask, receiver_only=True, donor_mapping='aperiodic-vertical')
            validate(worker)
            bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
            report = json.loads((worker / 'inspection/refinement.json').read_text())
            report['crown'].update(result)
            report.update(model_sha256=sha(worker / 'model.blend'), bark=bark)
            report['limitations'] = [note for note in report['limitations'] if not note.startswith('Off-map crown continuation')]
            report['limitations'].append('Off-map crown continuation uses irregular leaf clusters sampled from this same native crown. Every added face has inferred ownership; observed patches retain their in-map projection.')
            write_json(worker / 'inspection/refinement.json', report)
            audit(worker)
            write_json(receipt, dict(model_sha256=report['model_sha256'], previous_model_sha256=before,
                boundary_x=boundary, algorithm='world-ellipsoid-and-inferred-half-v2'))
        elif json.loads(receipt.read_text())['model_sha256'] != sha(worker / 'model.blend'):
            raise ValueError('Boundary revision changed')
        render_workspace(worker, 256, release_slot=False)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('masks', nargs='+', type=int, choices=[24, 40])
    parser.add_argument('--redo', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    for mask in args.masks:
        complete(mask)
