"""Append pinned changed receivers to a historical private scene, without publication."""
import argparse
import json
import shutil
import sys
from pathlib import Path

import bpy

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / 'refinement'))
sys.path.insert(0, str(HERE.parents[1] / 'refinement/blender'))
from approved_texture_stage import geometry, appearance
from catalog import reviewed_catalog, tree_workspace, scenery_workspace
from catalog_schema import source_for_part
from evidence_io import sha, write_json
from render_slots import acquire, release


def read(path):
    return json.loads(Path(path).read_text())


def checked(path, expected):
    path = Path(path)
    if sha(path) != expected:
        raise ValueError('Frozen input changed: ' + str(path))
    return path


def fingerprints(obj):
    return dict(geometry=geometry(obj), appearance=appearance(obj))


def validate_inputs(selection_path, selection, authority):
    for name, row in selection['metadata'].items():
        checked(selection_path.parent / name, row['sha256'])
        checked(row['source'], row['sha256'])
    for name, expected in selection['decision_files'].items():
        checked(selection_path.parent / name, expected)
    for path, expected in selection['source_evidence'].items():
        checked(path, expected)
    for record in selection['records']:
        if record['state_only']:
            continue
        checked(record['model'], record['model_sha256'])
        checked(record['geometry_model'], record['geometry_model_sha256'])
        for path, expected in record['evidence'].items():
            checked(path, expected)
    for row in authority['source_records']:
        refs = [row['frames'], row['source_mask_manifest'], row['mask_inventory'],
                *row['source_evidence']]
        for ref in refs:
            if ref:
                checked(ref['path'], ref['sha256'])
    for ref in authority.get('global_evidence', []):
        checked(ref['path'], ref['sha256'])


def append_receivers(model, names, collection, references):
    with bpy.data.libraries.load(str(model), link=False) as (source, target):
        if not set(names).issubset(source.objects):
            raise ValueError('Missing audited source receiver')
        # Blender replaces this list's strings with Object instances on exit.
        target.objects = list(names)
    imported = list(target.objects)
    for obj in imported:
        collection.objects.link(obj)
        parent = obj.parent
        while parent:
            if parent.type == 'MESH':
                raise ValueError('External mesh parent needs explicit receiver scope')
            if parent.name not in bpy.context.scene.objects:
                collection.objects.link(parent)
            parent = parent.parent
    bpy.context.view_layer.update()
    for name, obj in zip(names, imported):
        if fingerprints(obj) != references[name]:
            raise ValueError('Imported geometry, world transform or appearance differs: ' + name)
    return imported


def main(plan_path, output):
    plan = read(plan_path)
    selection_path = checked(plan['selection'], plan['selection_sha256'])
    selection = read(selection_path)
    authority_path = selection_path.parent / 'worker-source-authorities.json'
    authority = read(authority_path)
    authority_hash = sha(authority_path)
    if plan.get('source_authorities_sha256'):
        checked(authority_path, plan['source_authorities_sha256'])
    if authority['status'] != 'PASS' or authority['selection_sha256'] != sha(selection_path):
        raise ValueError('Worker source authority is stale')
    if output.exists():
        raise FileExistsError(output)
    if shutil.disk_usage(output.parent).free < plan['disk']['required_free_bytes_estimate']:
        raise ValueError('Insufficient approved disk budget headroom')
    old_assembly_path = Path(plan['base_scene']).parent / 'assembly.json'
    old = read(checked(old_assembly_path, plan['base_assembly_sha256']))
    checked(plan['base_scene'], plan['base_scene_sha256'])
    validate_inputs(selection_path, selection, authority)
    rows = {r['asset_id']: r for r in selection['records'] if not r['state_only']}
    updates = set(plan['added']) | {r['asset_id'] for r in plan['replacements']}
    refs = {}
    acquire()
    try:
        output.mkdir(parents=True)
        for asset in sorted(updates):
            record = rows[asset]
            audit = read(Path(record['worker']) / 'inspection/saved-model-audit.json')
            if audit['status'] != 'PASS' or audit['model_sha256'] != record['geometry_model_sha256']:
                raise ValueError('Saved worker audit differs: ' + asset)
            nodes = {source_for_part(p) for p in record['group']['parts']}
            if {r['source_node'] for r in audit['objects']} != nodes:
                raise ValueError('Worker does not cover exact group ownership: ' + asset)
            model = checked(record['model'], record['model_sha256'])
            bpy.ops.wm.open_mainfile(filepath=str(model))
            names = [r['object'] for r in audit['objects']]
            refs[asset] = {name: fingerprints(bpy.data.objects[name]) for name in names}
            texture = record['approved_texture']
            if texture and set(names) != set(texture['receiver_names']):
                raise ValueError('Texture receiver scope differs: ' + asset)
            checked(model, record['model_sha256'])
            print('PREFLIGHT', asset, flush=True)
        ground = plan['ground_candidate']
        bpy.ops.wm.open_mainfile(filepath=str(checked(ground['model'], ground['model_sha256'])))
        ground_names = [o.name for o in bpy.context.scene.objects
                        if o.type == 'MESH' and o.get('source_node') == 'ground']
        if len(ground_names) != 1:
            raise ValueError('Ground worker must contain one exact receiver')
        ground_refs = {n: fingerprints(bpy.data.objects[n]) for n in ground_names}
        write_json(output / 'source-fingerprints.json', dict(assets=refs, ground=ground_refs))
        bpy.ops.wm.open_mainfile(filepath=str(checked(plan['base_scene'], plan['base_scene_sha256'])))
        bpy.context.preferences.filepaths.save_version = 0
        scene = bpy.data.scenes['Croisement02 Refinement']
        bpy.context.window.scene = scene
        collection = bpy.data.collections['Croisement02 Working']
        for asset in sorted(updates):
            record = rows[asset]
            before = [o for o in collection.all_objects if o.type == 'MESH'
                      and o.get('asset_group') == asset]
            if bool(before) != (asset not in plan['added']):
                raise ValueError('Existing group scope differs from delta: ' + asset)
            for obj in before:
                bpy.data.objects.remove(obj, do_unlink=True)
            imported = append_receivers(checked(record['model'], record['model_sha256']),
                                        list(refs[asset]), collection, refs[asset])
            for obj in imported:
                if obj.get('asset_group') != asset:
                    raise ValueError('Imported asset identity differs')
                obj.hide_render = False
            print('IMPORTED', asset, flush=True)
        prior_ground = [o for o in collection.all_objects if o.type == 'MESH'
                        and o.get('source_node') == 'ground']
        if len(prior_ground) != 1:
            raise ValueError('Expected one prior ground receiver')
        bpy.data.objects.remove(prior_ground[0], do_unlink=True)
        new_ground = append_receivers(checked(ground['model'], ground['model_sha256']),
                                      ground_names, collection, ground_refs)
        for obj in new_ground:
            obj.hide_render = False
        states = {r['asset_id'] for r in selection['records'] if r['state_only']}
        visible = set(collection.all_objects)
        for obj in scene.objects:
            if obj.type == 'MESH':
                obj.hide_render = obj not in visible or obj.get('asset_group') in states
        expected_nodes = {source_for_part(p) for r in selection['records'] for p in r['group']['parts']}
        actual_nodes = {o.get('source_node') for o in collection.all_objects
                        if o.type == 'MESH' and o.get('source_node') != 'ground'}
        if actual_nodes != expected_nodes:
            raise ValueError('Whole scene source-node reconciliation differs')
        actual_groups = {o.get('asset_group') for o in collection.all_objects
                         if o.type == 'MESH' and not o.hide_render and o.get('source_node') != 'ground'}
        if actual_groups != set(rows):
            raise ValueError('Visible whole scene group reconciliation differs')
        expected = {o.name: fingerprints(o) for o in collection.all_objects if o.type == 'MESH'}
        validate_inputs(selection_path, selection, authority)
        checked(selection_path, plan['selection_sha256'])
        checked(authority_path, authority_hash)
        model = output / 'scene.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model), compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        bpy.context.window.scene = bpy.data.scenes['Croisement02 Refinement']
        for name, reference in expected.items():
            if fingerprints(bpy.data.objects[name]) != reference:
                raise ValueError('Saved scene altered receiver geometry or appearance: ' + name)
        validate_inputs(selection_path, selection, authority)
        checked(selection_path, plan['selection_sha256'])
        checked(authority_path, authority_hash)
        checked(plan['base_scene'], plan['base_scene_sha256'])
        selector_delta = []
        for asset, record in rows.items():
            group = record['group']
            try:
                current = tree_workspace(group['wood_mask']) if 'wood_mask' in group else scenery_workspace(asset)
                current_hash = sha(current / 'model.blend')
                if current_hash != record['geometry_model_sha256']:
                    selector_delta.append(dict(asset_id=asset, frozen_model_sha256=record['geometry_model_sha256'],
                                               current_worker=str(current), current_model_sha256=current_hash))
            except Exception as error:
                selector_delta.append(dict(asset_id=asset, current_selector_error=str(error)))
        write_json(output / 'reopened-preservation.json', dict(status='PASS',
                   model_sha256=sha(model), receivers=expected, geometry_appearance_world_transforms_unchanged=True))
        write_json(output / 'assembly.json', dict(status='private current126 spatial review; not publication',
                   model_sha256=sha(model), selection=str(selection_path), selection_sha256=sha(selection_path),
                   plan=str(plan_path), plan_sha256=sha(plan_path), source_authorities_sha256=sha(authority_path),
                   counts=plan['counts'], approved_texture_count=selection['approved_texture_count'],
                   ground=ground, reopened_preservation='PASS', new_model_bytes=model.stat().st_size,
                   base_scene_unchanged=True, later_selector_delta=selector_delta,
                   pending_replacements=plan['pending_replacements'], state_scope=plan['state_scope'],
                   source_roles_complete=False, visual_review='pending', publication='not performed'))
        print('SAVED', model, model.stat().st_size, flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.plan.resolve(), args.output.resolve())
