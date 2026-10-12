"""Freeze Lincoln inputs and inventory its editable volume reconstruction.

Run: blender --background --python level-editor/blender/lincoln/setup_scene.py
Then: python3 level-editor/blender/lincoln/setup_scene.py --illustrate
States: python3 level-editor/blender/lincoln/setup_scene.py --annotate-states
"""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/needs-review/lincoln-refinement'


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def json_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def recovered_uv(mesh):
    import math
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    uv = mesh.uv_layers.get('Source projection') or mesh.uv_layers.new(name='Source projection')
    for loop in mesh.loops:
        p = mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv = (p.x / 2304, 1 - (-p.y*sine-p.z*cosine)/3520)


def illustrate():
    from PIL import Image, ImageDraw
    inventory = json.loads((OUT / 'inventory/inventory.json').read_text())
    source = Image.open(inventory['source_image']).convert('RGB')
    overview = source.copy()
    draw = ImageDraw.Draw(overview)
    crops = OUT / 'inventory/crops'
    crops.mkdir(exist_ok=True)
    boxes = {}
    for obj in inventory['objects']:
        node = obj['source_node']
        if node == 'ground':
            continue
        box = obj['bounds_source_pixels']
        if node in boxes:
            old = boxes[node]
            box = [min(old[0], box[0]), min(old[1], box[1]),
                   max(old[2], box[2]), max(old[3], box[3])]
        boxes[node] = box
    index = []
    for node, box in sorted(boxes.items()):
        import math
        x0, y0, x1, y1 = box
        color = tuple(80 + c % 176 for c in hashlib.sha256(node.encode()).digest()[:3])
        draw.rectangle(box, outline=color, width=3)
        draw.text((x0 + 3, y0 + 3), node, fill='white', stroke_width=2, stroke_fill='black')
        region = [max(0, math.floor(x0)-45), max(0, math.floor(y0)-45),
                  min(source.width, math.ceil(x1)+45), min(source.height, math.ceil(y1)+45)]
        crop = source.crop(region)
        local = ImageDraw.Draw(crop)
        local.rectangle([x0-region[0], y0-region[1], x1-region[0], y1-region[1]], outline=color, width=2)
        local.text((5, 5), node, fill='white', stroke_width=2, stroke_fill='black')
        crop.save(crops / (node+'.png'))
        index.append({'source_node': node, 'bounds': box, 'crop': 'crops/'+node+'.png', 'crop_region': region})
    overview.save(OUT / 'inventory/source-overview-labeled.png')
    (OUT / 'inventory/crops.json').write_text(json.dumps(index, indent=2)+'\n')
    print(json.dumps({'overview': str(OUT / 'inventory/source-overview-labeled.png'), 'crops': len(index)}))


def setup():
    import bpy
    sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
    from setup_map import setup_map
    from refinement_inventory import inventory
    baseline = OUT / 'baseline'
    baseline.mkdir(parents=True, exist_ok=False)
    sources = {
        'lincoln-volumes.scene.json': ROOT / 'level-editor/library/scenes/lincoln-volumes.scene.json',
        'lincoln-volumes.scene.glb': ROOT / 'level-editor/library/scenes/lincoln-volumes.scene.glb',
        'lincoln.map.png': ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels/Day/lincoln.map.png',
    }
    hashes = {}
    for filename, source in sources.items():
        shutil.copy2(source, baseline / filename)
        hashes[filename] = {'original_path': str(source), 'sha256': digest(source)}
        assert digest(baseline / filename) == hashes[filename]['sha256']
    blend = baseline / 'lincoln-baseline.blend'
    result = setup_map(baseline / 'lincoln-volumes.scene.json', blend)
    scene = bpy.context.scene
    working = bpy.data.collections['Lincoln Working']
    normalized = {}
    for obj in working.all_objects:
        node = obj['source_obstacle']
        # The volume exporter names some native obstacles terrace-N; the index is the identity.
        if node.startswith('terrace-'):
            normalized[node] = 'building-' + node.removeprefix('terrace-')
            node = normalized[node]
        obj['source_node'] = node
    scene['source_artwork'] = str(baseline / 'lincoln.map.png')
    scene['source_artwork_sha256'] = hashes['lincoln.map.png']['sha256']
    scene['frozen_baseline'] = True
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(blend))
    report = inventory(OUT / 'inventory', collection_name=working.name, map_name='lincoln',
                       source_path=baseline / 'lincoln.map.png')
    path = OUT / 'inventory/inventory.json'
    data = json.loads(path.read_text())
    for record in data['objects']:
        obj = bpy.data.objects[record['object']]
        geometry = {'vertices': [list(v.co) for v in obj.data.vertices],
                    'faces': [list(p.vertices) for p in obj.data.polygons]}
        uvs = {layer.name: [list(loop.uv) for loop in layer.data] for layer in obj.data.uv_layers}
        record.update(parent=obj.parent.name if obj.parent else None,
                      source_obstacle=obj.get('source_obstacle'), editor_name=obj.name,
                      matrix_world=[list(row) for row in obj.matrix_world],
                      matrix_local=[list(row) for row in obj.matrix_local],
                      matrix_parent_inverse=[list(row) for row in obj.matrix_parent_inverse],
                      geometry=geometry, uv_layers=uvs, geometry_sha256=json_hash(geometry),
                      uv_sha256=json_hash(uvs),
                      materials=[slot.material.name if slot.material else None for slot in obj.material_slots],
                      polygon_material_indices=[p.material_index for p in obj.data.polygons],
                      modifiers=[{'name': m.name, 'type': m.type} for m in obj.modifiers],
                      custom_properties={key: obj[key] for key in obj.keys()},
                      animation_present=obj.animation_data is not None,
                      mask_associations=[], state_associations=[],
                      association_status='Requires native patch/state reconciliation')
    data['source_inputs'] = hashes
    data['normalized_node_ids'] = normalized
    data['source_blend_sha256'] = digest(blend)
    data['source_dimensions'] = json.loads((baseline / 'lincoln-volumes.scene.json').read_text())['size']
    data['cameras'] = [{'name': obj.name, 'matrix_world': [list(row) for row in obj.matrix_world],
                        'kind': obj.data.type, 'ortho_scale': obj.data.ortho_scale,
                        'clip_start': obj.data.clip_start, 'clip_end': obj.data.clip_end}
                       for obj in scene.objects if obj.type == 'CAMERA']
    data['materials'] = [{'name': mat.name, 'use_nodes': mat.use_nodes,
                         'images': [{'name': n.image.name, 'filepath': n.image.filepath,
                                     'packed': n.image.packed_file is not None}
                                    for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
                                   if mat.node_tree else []} for mat in bpy.data.materials]
    data['limitations'] = ['Imported synthesized atlas is not source-only ownership evidence.',
                           'Static source nodes require visual logical grouping and native patch/state reconciliation.',
                           'No geometry refinement or approval has occurred.']
    path.write_text(json.dumps(data, indent=2)+'\n')
    hashes[blend.name] = {'sha256': digest(blend)}
    (baseline / 'manifest.json').write_text(json.dumps({'version': 1, 'map': 'lincoln',
        'inputs': hashes, 'setup': result, 'script': str(Path(__file__).resolve()),
        'script_sha256': digest(__file__), 'blender_version': bpy.app.version_string}, indent=2)+'\n')
    print(json.dumps(report))


def annotate_states(path=None):
    path = Path(path) if path else OUT / 'inventory/inventory.json'
    data = json.loads(path.read_text())
    layer_path = OUT / 'source-states/layers.json'
    layers = json.loads(layer_path.read_text())
    if layers['map'].casefold() != 'lincoln':
        raise ValueError('State manifest belongs to another map')
    associations = {}
    for origin, patches in [('base', layers['patches']), ('mission', layers.get('mission_patches', []))]:
        for patch in patches:
            for key, state, masks_key in [('sight_before', 'covered', 'old_masks'), ('sight_after', 'revealed', 'new_masks')]:
                for node in patch.get(key, []):
                    associations.setdefault(node, []).append({
                        'patch_id': patch['id'], 'patch_name': patch['name'], 'origin': origin,
                        'state': state, 'native_membership': key,
                        'state_mask_refs': patch['state'].get(masks_key, []),
                        'state_mask_global_indices': patch['native_mask_global_indices'][masks_key],
                        'mask_ownership_status': 'Patch-state association only; per-receiver pixel ownership requires validation',
                    })
    for obj in data['objects']:
        obj['state_associations'] = associations.get(obj['source_node'], [])
        obj['association_status'] = ('Native obstacle membership recorded; mask receiver ownership unvalidated'
                                     if obj['state_associations'] else 'No native patch obstacle membership')
    data['patch_manifest'] = str(layer_path)
    data['patch_manifest_sha256'] = digest(layer_path)
    data['source_inputs']['setup_scene.py'] = {'sha256': digest(__file__)}
    data['patch_inventory_status'] = 'Native state obstacle membership supplied; visual receiver and mask ownership review required'
    path.write_text(json.dumps(data, indent=2)+'\n')
    native_count = len(json.loads((OUT / 'source-states/level.json').read_text())['sight_obstacles'])
    recovery_path = OUT / 'baseline/recovery-manifest.json'
    recovered = json.loads(recovery_path.read_text())['recovered_nodes'] if recovery_path.exists() else []
    original_path = OUT / 'inventory/imported-inventory.json'
    original = json.loads((original_path if original_path.exists() else path).read_text())
    previous = {obj['source_obstacle']: obj for obj in original['objects']}
    unchanged = all(all(obj[key] == previous[obj['source_obstacle']][key]
                        for key in ['geometry_sha256', 'uv_sha256', 'matrix_world', 'matrix_local', 'matrix_parent_inverse'])
                    for obj in data['objects'] if obj['source_node'] not in recovered)
    coverage = {obj['source_node'] for obj in data['objects']} == {f'building-{i:03}' for i in range(native_count)} | {'ground'}
    frozen = json.loads((OUT / 'baseline/manifest.json').read_text())
    intact = digest(OUT / 'baseline/lincoln-baseline.blend') == frozen['inputs']['lincoln-baseline.blend']['sha256']
    validation = {'status': 'PASS' if unchanged and coverage and intact else 'FAIL',
                  'existing_geometry_uv_transforms_unchanged': unchanged, 'all_native_obstacles_present': coverage,
                  'frozen_import_hash_unchanged': intact, 'recovered_nodes': recovered,
                  'state_associated_nodes': len(associations), 'mesh_count': len(data['objects'])}
    (OUT / 'inventory/setup-validation.json').write_text(json.dumps(validation, indent=2)+'\n')
    if validation['status'] != 'PASS':
        raise ValueError(validation)
    shutil.copy2(__file__, OUT / 'baseline/setup_scene.py')
    print(json.dumps(validation))


if __name__ == '__main__':
    if '--illustrate' in sys.argv:
        illustrate()
    elif '--annotate-states' in sys.argv:
        annotate_states()
    else:
        setup()
