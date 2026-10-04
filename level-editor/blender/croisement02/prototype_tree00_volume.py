"""Isolated crown cleanup experiment; earlier approved workers remain frozen."""
import argparse
import json
import shutil
import sys
from pathlib import Path

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, tree_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from refinement_workspace import prepare, modified, validate
from curved_crown_geometry import build
from complete_northern_caps import cap
from audit_candidates import audit
from render_tree import render_workspace


def main(destination, mask=0, interior_clusters=600, root_completion_base=None, branch_clumps=False, approved_base=None):
    worker = destination / 'assets' / f'croisement02-tree-{mask:02}'
    if worker.exists():
        raise ValueError('Use a fresh prototype destination')
    old = approved_base or tree_workspace(mask)
    if old.name != worker.name:
        raise ValueError('Approved base asset does not match the requested crown')
    old_hash = sha(old / 'model.blend')
    decisions={r['asset_id']:r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    approval=decisions.get(old.name,{})
    if approval.get('decision')!='approved' or approval.get('model_sha256')!=old_hash:
        raise ValueError('Cleanup rollout requires a current, explicitly approved geometry base')
    approved = old
    root_base = None
    if root_completion_base is not None:
        from canopy_root_base import validate as validate_root_base
        root_base = validate_root_base(root_completion_base, approved)
        old = root_completion_base
    old_report = json.loads((old / 'inspection/refinement.json').read_text())
    cfg = json.loads((old / 'workspace.json').read_text())
    source_row = next(r for r in json.loads((OUT / 'forest-v4-sources/manifest.json').read_text()) if r['mask'] == mask)
    source_packet = Path(old_report.get('source_packet', source_row['packet']))
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(old / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        prepare(worker, asset_id=old.name, scene_name=cfg['scene_name'], collection_name=cfg['collection_name'],
                source_path=old / 'reference/source.png', grouping_manifest=old / 'reference/grouping.json',
                inventory_path=old / 'reference/inventory.json', review_path=old / 'reference/grouping-review.json',
                source_mask_manifest=old / 'source-masks.json', width=384, height=384,
                framing_padding=1.7, lighting=cfg['lighting'])
        bpy.ops.wm.open_mainfile(filepath=str(old / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        objects = [o for o in bpy.data.collections[cfg['collection_name']].all_objects
                   if o.type == 'MESH' and o.get('asset_group') == old.name]
        crown = next(o for o in objects if o.get('projection_component') == 'crown')
        from approved_texture_stage import geometry, appearance
        preserved = {o.name: dict(geometry=geometry(o), appearance=appearance(o))
                     for o in objects if o != crown}
        inspection = worker / 'inspection'
        local_source = inspection / 'source-packet'
        local_source.mkdir(parents=True)
        packet = json.loads(source_packet.read_text())
        if branch_clumps:
            supports_path=Path(source_row['packet'])
            supports=json.loads(supports_path.read_text())
            packet['branch_supports']=[dict(seed=lobe['seed'],bbox=lobe['bbox']) for lobe in supports['lobes']]
            packet['branch_supports_source_sha256']=sha(supports_path)
            shutil.copy2(supports_path,local_source/'branch-supports.json')
        shutil.copy2(source_packet.parent / 'complete-source.png', local_source / 'complete-source.png')
        for lobe in packet['lobes']:
            original = Path(lobe['image'])
            shutil.copy2(original, local_source / original.name)
            lobe['image'] = str(local_source / original.name)
        packet_path = local_source / 'partition.json'
        write_json(packet_path, packet)
        result = build(crown, packet, source_row['ground_y'], interior_clusters=interior_clusters, branch_clumps=branch_clumps)
        native_alpha = np.asarray(Image.open(local_source / 'complete-source.png'))[:, :, 3]
        north_row = -packet['native_bbox'][1]
        north_contact = (int(np.count_nonzero(native_alpha[north_row] > 127))
                         if 0 <= north_row < native_alpha.shape[0] else 0)
        result['northern_completion'] = (cap(crown, packet_path, mask, inspection) if north_contact >= 8
            else dict(status='not applicable; native crown does not reach north map edge', contact_pixels=north_contact))
        result['material_source_roles'] = [dict(slot=i, material=m.name,
            source_role=('source-projected native RGB and alpha' if i in (0, 5)
                         else 'inferred placement and palette from own native crown'),
            faces=sum(p.material_index == i for p in crown.data.polygons))
            for i, m in enumerate(crown.data.materials) if any(p.material_index == i for p in crown.data.polygons)]
        result['native_source_sha256'] = sha(local_source / 'complete-source.png')
        saved = {o.name: o.data.copy() for o in objects}
        for mesh in saved.values():
            for i, mat in enumerate(mesh.materials):
                mesh.materials[i] = mat.copy()
        bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
        modified(worker)
        for obj in objects:
            obj.data = saved[obj.name]
        validate(worker)
        bpy.ops.wm.save_as_mainfile(filepath=str(worker / 'model.blend'))
        report = dict(old_report, model_sha256=sha(worker / 'model.blend'), crown=result,
                      source_packet=str(packet_path), status='New cleanup prototype; geometry approval pending')
        report['limitations'] = [('Observed fragments retain native source rays across inferred branch-scale clumps; rear foliage uses only this native canopy artwork.'
                                 if branch_clumps else
                                 'Observed patches are jittered across an inferred ellipsoid; rear foliage uses only this native canopy artwork.'),
            'Northern continuation is added only where native alpha reaches the map edge. Only the two permitted Leicester references inform construction.',
            'Earlier approval does not apply to this rebuilt crown. No texture API generation or publication performed.']
        write_json(inspection / 'refinement.json', report)
        audit(worker)
        render_workspace(worker,384,release_slot=False)
        coverage = json.loads((inspection / 'source-coverage/report.json').read_text())
        bounds = json.loads((inspection / 'actual-materials/opacity-bounds.json').read_text())
        if coverage['intersection_over_union'] < .95:
            raise ValueError('Prototype failed native-front coverage')
        if any(c['depth_width_ratio'] < 1.0 for c in bounds['crowns']):
            raise ValueError('Prototype has insufficient physical leaf depth')
        if sha(approved / 'model.blend') != old_hash:
            raise ValueError('Approved model changed')
        current = {o.name: dict(geometry=geometry(o), appearance=appearance(o))
                   for o in bpy.data.collections[cfg['collection_name']].all_objects
                   if o.type == 'MESH' and o.get('asset_group') == old.name
                   and o.get('projection_component') != 'crown'}
        if current != preserved:
            raise ValueError('Crown cleanup changed the preserved wood or root geometry/materials')
        if root_base is not None and validate_root_base(old, approved) != root_base:
            raise ValueError('Root base changed during crown cleanup')
        write_json(inspection / 'prototype-preservation.json', dict(previous_worker=str(approved),
            previous_model_sha256=old_hash, previous_model_unchanged=True, model_sha256=sha(worker / 'model.blend'),
            approval='pending', texture_generation='not performed', root_completion_base=root_base,
            non_crown_geometry_and_materials_preserved=True, preserved_non_crown=preserved))
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--mask',type=int,default=0,help='Native wood mask; one isolated candidate per invocation')
    parser.add_argument('--interior-clusters', type=int, default=600, help='Private inferred volume density experiment')
    parser.add_argument('--root-completion-base', type=Path, help='Reviewed private tree15 root addition to preserve')
    parser.add_argument('--branch-clumps', action='store_true', help='Private irregular branch-scale volume experiment')
    parser.add_argument('--approved-base', type=Path, help='Explicit approved original when a pending candidate is already selected')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.destination.resolve(),args.mask,args.interior_clusters,
         args.root_completion_base.resolve() if args.root_completion_base else None,args.branch_clumps,
         args.approved_base.resolve() if args.approved_base else None)
