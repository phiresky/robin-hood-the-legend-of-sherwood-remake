"""Assemble current workers for private full-scene review, without publication."""
import argparse
import json
import math
import shutil
import sys
import uuid
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, reviewed_catalog, tree_workspace, scenery_workspace
from evidence_io import sha, write_json, digest
from render_slots import acquire, release
from tree_geometry import SIN, RAY
from catalog_schema import source_for_part


def signature(obj):
    return digest(dict(vertices=[list(v.co) for v in obj.data.vertices],
                       faces=[list(p.vertices) for p in obj.data.polygons],
                       uv={layer.name: [list(v.uv) for v in layer.data] for layer in obj.data.uv_layers}))


def render_review(scene, destination, model_hash):
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = 4
    scene.cycles.transparent_max_bounces = 64
    scene.render.resolution_x = 1024
    scene.render.resolution_y = 768
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = 'PNG'
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    data = bpy.data.cameras.new('Integration review camera')
    data.type = 'ORTHO'
    data.ortho_scale = 2450
    data.clip_end = 20000
    camera = bpy.data.objects.new(data.name, data)
    scene.collection.objects.link(camera)
    scene.camera = camera
    center = Vector((896, -576 / SIN, 80))
    views = []
    for index in range(8):
        angle = index * math.tau / 8
        direction = Vector((math.sin(angle) * math.cos(math.radians(30)),
                            -math.cos(angle) * math.cos(math.radians(30)), math.sin(math.radians(30))))
        camera.location = center + direction * 5000
        camera.rotation_euler = (center - camera.location).to_track_quat('-Z', 'Y').to_euler()
        # Include inferred off-map geometry at every rotation, not just map bounds.
        rotation = camera.rotation_euler.to_matrix()
        right, up = rotation.col[0], rotation.col[1]
        extent_x = extent_y = 0.0
        for obj in scene.objects:
            if obj.type != 'MESH' or obj.hide_render:
                continue
            for corner in obj.bound_box:
                delta = obj.matrix_world @ Vector(corner) - center
                extent_x = max(extent_x, abs(delta.dot(right)))
                extent_y = max(extent_y, abs(delta.dot(up)))
        aspect = scene.render.resolution_x / scene.render.resolution_y
        data.ortho_scale = max(2 * extent_x, 2 * extent_y * aspect) * 1.08
        path = destination / f'view-{index}.png'
        scene.render.filepath = str(path)
        bpy.ops.render.render(write_still=True, scene=scene.name)
        views.append(dict(image=path.name, sha256=sha(path), location=list(camera.location),
                          rotation=list(camera.rotation_euler), ortho_scale=data.ortho_scale))
    sheet = Image.new('RGB', (2048, 768))
    for index in range(8):
        im = Image.open(destination / f'view-{index}.png').convert('RGB')
        im.thumbnail((512, 384))
        sheet.paste(im, (index % 4 * 512, index // 4 * 384))
    sheet.save(destination / 'sheet.png')
    write_json(destination / 'render-evidence.json', dict(model_sha256=model_hash,
        views=views, sheet_sha256=sha(destination / 'sheet.png'), visual_review='pending'))


def main(destination=None, texture_decisions=None):
    explicit_destination = destination is not None
    destination = destination or OUT / 'integration-review'
    if explicit_destination and destination.exists():
        raise ValueError("Explicit staging destination already exists")
    if (destination / 'scene.blend').exists():
        destination.rename(destination.with_name('integration-review-archive-'+uuid.uuid4().hex[:8]))
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(reviewed_catalog(), destination / 'catalog.json')
    catalog = json.loads((destination / 'catalog.json').read_text())
    selected = {}
    models = {group['id']: (tree_workspace(group['wood_mask']) if 'wood_mask' in group
              else scenery_workspace(group['id'])) / 'model.blend'
              for group in catalog['groups'] if not group.get('state_only')}
    if texture_decisions:
        from approved_texture_stage import select, inspect, geometry, appearance
        selected = select(texture_decisions, models)
        shutil.copy2(texture_decisions, destination / 'texture-decisions.json')
    acquire()
    try:
        if selected:
            inspect(selected, models)
            write_json(destination / 'selected-texture-approvals.json', selected)
        bpy.ops.wm.open_mainfile(filepath=str(OUT / 'forest-v4-input.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        scene = bpy.data.scenes['Croisement02 Refinement']
        bpy.context.window.scene = scene
        collection = bpy.data.collections['Croisement02 Working']
        owners = {source_for_part(p): group for group in catalog['groups'] for p in group['parts']}
        for obj in list(collection.all_objects):
            if obj.type == 'MESH' and obj.get('source_node') in owners:
                group = owners[obj['source_node']]
                obj['asset_group'] = group['id']
                obj['asset_name'] = group['name']
                obj.hide_render = bool(group.get('state_only'))
        latest = {r['asset_id']: r for r in json.loads((OUT / 'user-feedback.json').read_text())['records']}
        records = []
        for group in catalog['groups']:
            if group.get('state_only'):
                records.append(dict(id=group['id'], role='state metadata; hidden in base scenery', parts=group['parts']))
                continue
            worker = tree_workspace(group['wood_mask']) if 'wood_mask' in group else scenery_workspace(group['id'])
            audit_path = worker / 'inspection/saved-model-audit.json'
            if not audit_path.exists():
                records.append(dict(id=group['id'], role='unrefined context proxy', parts=group['parts']))
                continue
            audit = json.loads(audit_path.read_text())
            expected_parts = {source_for_part(p) for p in group['parts']}
            if {r['source_node'] for r in audit['objects']} != expected_parts:
                records.append(dict(id=group['id'], role='unrefined context proxy; prior worker source ownership is stale', parts=group['parts']))
                continue
            model = worker / 'model.blend'
            model_hash = sha(model)
            if audit['status'] != 'PASS' or audit['model_sha256'] != model_hash:
                raise ValueError('Worker changed during staging: ' + group['id'])
            names = [r['object'] for r in audit['objects']]
            texture = selected.get(group['id'])
            geometry_model_hash = model_hash
            if texture:
                if set(names) != set(texture['objects']):
                    raise ValueError('Texture scope differs from audited asset: ' + group['id'])
                model = Path(texture['model'])
                model_hash = sha(model)
            old_objects = [o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == group['id']]
            with bpy.data.libraries.load(str(model), link=False) as (source, target):
                if not set(names) <= set(source.objects):
                    raise ValueError('Audited objects missing from model')
                target.objects = list(names)
            imported = list(target.objects)
            for obj in imported:
                collection.objects.link(obj)
            bpy.context.view_layer.update()
            evidence = []
            for original_name, obj in zip(names, imported):
                if texture:
                    reference = texture['objects'][original_name]
                    if geometry(obj) != reference['geometry'] or appearance(obj) != reference['appearance']:
                        raise ValueError('Imported approved texture or geometry changed: ' + original_name)
                before = signature(obj)
                matrix = obj.matrix_world.copy()
                if texture:
                    # Retain the reviewed transform chain. Flattening it forces
                    # a matrix decomposition and changes float32 rotations.
                    ancestor = obj.parent
                    while ancestor is not None:
                        if ancestor.type == 'MESH':
                            raise ValueError('Texture receiver has a mesh parent outside its scope')
                        if ancestor.name not in scene.objects:
                            collection.objects.link(ancestor)
                        ancestor = ancestor.parent
                else:
                    obj.parent = None
                    obj.matrix_world = matrix
                obj.hide_render = False
                if obj.get('asset_group') != group['id'] or signature(obj) != before:
                    raise ValueError('Imported asset scope or surface changed')
                evidence.append(dict(source_node=obj['source_node'], component=obj.get('projection_component'),
                                     surface_sha256=before, matrix_world=[list(r) for r in matrix],
                                     imported_name=obj.name,
                                     texture_appearance_sha256=appearance(obj) if texture else None,
                                     texture_geometry_sha256=geometry(obj) if texture else None))
            if {o['source_node'] for o in imported} != expected_parts:
                raise ValueError('Imported ownership differs from current catalog: ' + group['id'])
            for obj in old_objects:
                bpy.data.objects.remove(obj, do_unlink=True)
            decision = latest.get(group['id'], {})
            approved = decision.get('decision') == 'approved' and decision.get('model_sha256') == geometry_model_hash
            correction_path = worker / 'inspection/feedback-revision-1.json'
            if not approved and decision.get('decision') == 'approved' and correction_path.exists():
                correction = json.loads(correction_path.read_text())
                approved = (correction['model_sha256'] == model_hash
                            and correction['before_model_sha256'] == decision['model_sha256']
                            and correction['before_geometry_sha256'] == correction['geometry_sha256'])
            records.append(dict(id=group['id'], worker=str(worker), model_sha256=model_hash,
                role='geometry and texture approved' if texture else ('geometry approved' if approved else 'unapproved candidate'),
                geometry_model_sha256=geometry_model_hash, texture_approval=texture['decision']['review_revision'] if texture else None,
                objects=evidence))
            print('STAGED', group['id'], flush=True)
        # Only the working collection is part of this review; baseline reference
        # objects elsewhere in the file must not double the visible geometry.
        visible = set(collection.all_objects)
        for obj in scene.objects:
            if obj.type == 'MESH' and obj not in visible:
                obj.hide_render = True
        parts = {o['source_node'] for o in collection.all_objects if o.type == 'MESH' and o.get('source_node') != 'ground'}
        if parts != set(owners):
            raise ValueError('Full-scene native part reconciliation failed')
        model = destination / 'scene.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model))
        if selected:
            bpy.ops.wm.open_mainfile(filepath=str(model))
            scene = bpy.data.scenes['Croisement02 Refinement']
            bpy.context.window.scene = scene
            for record in records:
                if not record.get('texture_approval'):
                    continue
                for evidence in record['objects']:
                    obj = bpy.data.objects[evidence['imported_name']]
                    if (appearance(obj) != evidence['texture_appearance_sha256'] or
                            geometry(obj) != evidence['texture_geometry_sha256']):
                        raise ValueError('Saved integration changed approved asset: ' + record['id'])
            if len([r for r in records if r.get('texture_approval')]) != len(selected):
                raise ValueError('Not every selected texture was staged')
            select(texture_decisions, models)
        report = dict(approved_textures=len(selected), approved_texture_import_preservation='PASS' if selected else 'not requested', status='private integration review; not published', model_sha256=sha(model),
            catalog_sha256=sha(destination / 'catalog.json'), assets=records,
            native_parts=sum(p.startswith('building-') for p in parts),
            authored_parts=sum(p.startswith(('foliage-', 'scenery-')) for p in parts),
            remaining=['Terrain still retains its source artwork; foreground removal and hidden-ground completion are pending.',
                       'Unrefined context proxies and unapproved candidates are included for spatial review only.',
                       'Mask-only shrubs/grass, remaining boundary trees, and animated mission states are not yet integrated.'])
        write_json(destination / 'assembly.json', report)
        render_review(scene, destination, report['model_sha256'])
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, help='Fresh isolated destination')
    parser.add_argument('--approved-textures', type=Path, help='Strict texture decisions file')
    parser.add_argument('--render-existing', type=Path, help='Render a pinned private stage into a fresh --output directory')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else [])
    if args.render_existing:
        if args.output is None or args.output.exists():
            raise ValueError('Rerender requires a fresh output directory')
        report = json.loads((args.render_existing / 'assembly.json').read_text())
        model = args.render_existing / 'scene.blend'
        if sha(model) != report['model_sha256']:
            raise ValueError('Pinned integration model changed')
        acquire()
        try:
            bpy.ops.wm.open_mainfile(filepath=str(model))
            scene = bpy.data.scenes['Croisement02 Refinement']
            bpy.context.window.scene = scene
            args.output.mkdir(parents=True)
            write_json(args.output / 'source-stage.json', dict(stage=str(args.render_existing.resolve()),
                assembly_sha256=sha(args.render_existing / 'assembly.json'), model_sha256=sha(model)))
            render_review(scene, args.output, report['model_sha256'])
        finally:
            release()
    else:
        main(args.output, args.approved_textures)
