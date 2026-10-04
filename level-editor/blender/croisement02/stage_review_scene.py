"""Assemble current workers for private full-scene review, without publication."""
import json
import math
import shutil
import sys
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


def signature(obj):
    return digest(dict(vertices=[list(v.co) for v in obj.data.vertices],
                       faces=[list(p.vertices) for p in obj.data.polygons],
                       uv={layer.name: [list(v.uv) for v in layer.data] for layer in obj.data.uv_layers}))


def main():
    destination = OUT / 'integration-review'
    destination.mkdir(exist_ok=True)
    shutil.copy2(reviewed_catalog(), destination / 'catalog.json')
    catalog = json.loads((destination / 'catalog.json').read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(OUT / 'forest-v4-input.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        scene = bpy.data.scenes['Croisement02 Refinement']
        bpy.context.window.scene = scene
        collection = bpy.data.collections['Croisement02 Working']
        owners = {f"building-{p['obstacle']:03}": group for group in catalog['groups'] for p in group['parts']}
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
            model = worker / 'model.blend'
            model_hash = sha(model)
            if audit['status'] != 'PASS' or audit['model_sha256'] != model_hash:
                raise ValueError('Worker changed during staging: ' + group['id'])
            names = [r['object'] for r in audit['objects']]
            old_objects = [o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == group['id']]
            with bpy.data.libraries.load(str(model), link=False) as (source, target):
                if not set(names) <= set(source.objects):
                    raise ValueError('Audited objects missing from model')
                target.objects = names
            imported = list(target.objects)
            for obj in imported:
                collection.objects.link(obj)
            bpy.context.view_layer.update()
            evidence = []
            for obj in imported:
                before = signature(obj)
                matrix = obj.matrix_world.copy()
                obj.parent = None
                obj.matrix_world = matrix
                obj.hide_render = False
                if obj.get('asset_group') != group['id'] or signature(obj) != before:
                    raise ValueError('Imported asset scope or surface changed')
                evidence.append(dict(source_node=obj['source_node'], component=obj.get('projection_component'),
                                     surface_sha256=before, matrix_world=[list(r) for r in matrix]))
            if {o['source_node'] for o in imported} != {f"building-{p['obstacle']:03}" for p in group['parts']}:
                raise ValueError('Imported ownership differs from current catalog: ' + group['id'])
            for obj in old_objects:
                bpy.data.objects.remove(obj, do_unlink=True)
            decision = latest.get(group['id'], {})
            approved = decision.get('decision') == 'approved' and decision.get('model_sha256') == model_hash
            correction_path = worker / 'inspection/feedback-revision-1.json'
            if not approved and decision.get('decision') == 'approved' and correction_path.exists():
                correction = json.loads(correction_path.read_text())
                approved = (correction['model_sha256'] == model_hash
                            and correction['before_model_sha256'] == decision['model_sha256']
                            and correction['before_geometry_sha256'] == correction['geometry_sha256'])
            records.append(dict(id=group['id'], worker=str(worker), model_sha256=model_hash,
                role='geometry approved' if approved else 'unapproved candidate', objects=evidence))
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
        report = dict(status='private integration review; not published', model_sha256=sha(model),
            catalog_sha256=sha(destination / 'catalog.json'), assets=records, native_parts=len(parts),
            remaining=['Terrain still retains its source artwork; foreground removal and hidden-ground completion are pending.',
                       'Unrefined context proxies and unapproved candidates are included for spatial review only.',
                       'Mask-only shrubs/grass, remaining boundary trees, and animated mission states are not yet integrated.'])
        write_json(destination / 'assembly.json', report)
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
        write_json(destination / 'render-evidence.json', dict(model_sha256=report['model_sha256'],
            views=views, sheet_sha256=sha(destination / 'sheet.png'), visual_review='pending'))
    finally:
        release()


if __name__ == '__main__':
    main()
