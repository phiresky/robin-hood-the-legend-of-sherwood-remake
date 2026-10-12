"""Freeze Nottingham inputs and inventory its editable volume reconstruction.

Run: blender --background --python level-editor/blender/nottingham/setup_scene.py
Then: python3 level-editor/blender/nottingham/setup_scene.py --illustrate
Recovery: blender --background --python level-editor/blender/nottingham/setup_scene.py -- --recover
States: python3 level-editor/blender/nottingham/setup_scene.py --annotate-states
"""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/needs-review/nottingham-refinement'


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
        'nottingham-volumes.scene.json': ROOT / 'level-editor/library/scenes/nottingham-volumes.scene.json',
        'nottingham-volumes.scene.glb': ROOT / 'level-editor/library/scenes/nottingham-volumes.scene.glb',
        'Nottingham.map.png': ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels/Day/Nottingham.map.png',
    }
    hashes = {}
    for filename, source in sources.items():
        shutil.copy2(source, baseline / filename)
        hashes[filename] = {'original_path': str(source), 'sha256': digest(source)}
        assert digest(baseline / filename) == hashes[filename]['sha256']
    blend = baseline / 'nottingham-baseline.blend'
    result = setup_map(baseline / 'nottingham-volumes.scene.json', blend)
    scene = bpy.context.scene
    working = bpy.data.collections['nottingham Working']
    for obj in working.all_objects:
        obj['source_node'] = obj['source_obstacle']
    scene['source_artwork'] = str(baseline / 'Nottingham.map.png')
    scene['source_artwork_sha256'] = hashes['Nottingham.map.png']['sha256']
    scene['frozen_baseline'] = True
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(blend))
    report = inventory(OUT / 'inventory', collection_name=working.name, map_name='nottingham',
                       source_path=baseline / 'Nottingham.map.png')
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
    data['source_blend_sha256'] = digest(blend)
    data['source_dimensions'] = json.loads((baseline / 'nottingham-volumes.scene.json').read_text())['size']
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
    (baseline / 'manifest.json').write_text(json.dumps({'version': 1, 'map': 'nottingham',
        'inputs': hashes, 'setup': result, 'script': str(Path(__file__).resolve()),
        'script_sha256': digest(__file__), 'blender_version': bpy.app.version_string}, indent=2)+'\n')
    print(json.dumps(report))


def recover():
    """Recover the native thin slab omitted by the volume export height filter."""
    import bpy
    import math
    from mathutils import Vector
    from mathutils.geometry import tessellate_polygon
    sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
    from refinement_inventory import inventory
    baseline = OUT / 'baseline'
    target = baseline / 'nottingham-recovered.blend'
    if target.exists():
        raise FileExistsError(target)
    source = ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels/nottingham.rhp.json'
    shutil.copy2(source, baseline / source.name)
    native = json.loads(source.read_text())
    bpy.ops.wm.open_mainfile(filepath=str(baseline / 'nottingham-baseline.blend'))
    bpy.context.window.scene = bpy.data.scenes['nottingham Refinement']
    collection = bpy.data.collections['nottingham Working']
    for obj in collection.objects:
        if obj.get('source_node') == 'terrace-366':
            obj['source_node'] = 'building-366'
    obstacle = native['sight_obstacles'][82]
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    bottom = [(p['x'], -p['y']/sine, p['z_bottom']/cosine) for p in obstacle['points']]
    top = [(p['x'], -p['y']/sine, p['z_top']/cosine) for p in obstacle['points']]
    n = len(top)
    vertices = bottom + top
    # The top and side receivers follow the volume reconstruction convention.
    # Retain exact native point positions; the omitted sub-two-pixel thickness matters.
    area = sum(bottom[i][0]*bottom[(i+1)%n][1] - bottom[(i+1)%n][0]*bottom[i][1] for i in range(n))
    faces = []
    for i in range(n):
        j = (i+1)%n
        face = (i,j,j+n,i+n)
        faces.append(face if area > 0 else tuple(reversed(face)))
    vectors = [Vector(p) for p in top]
    for tri in tessellate_polygon([vectors]):
        ids = tuple((v if isinstance(v, int) else min(range(n), key=lambda i: (vectors[i]-v).length_squared))+n for v in tri)
        a,b,c = [Vector(vertices[i]) for i in ids]
        faces.append(ids if (b-a).cross(c-a).z > 0 else tuple(reversed(ids)))
    mesh = bpy.data.meshes.new('Recovered native slab 082')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    recovered_uv(mesh)
    obj = bpy.data.objects.new('building-082', mesh)
    collection.objects.link(obj)
    obj['source_node'] = 'building-082'
    obj['source_obstacle'] = 'building-082'
    obj['recovery_reason'] = 'Native thickness below volume exporter MIN_HEIGHT=2'
    obj['native_obstacle_index'] = 82
    material = bpy.data.materials.new('Recovered source geometry - unknown neutral')
    material.diffuse_color = (0.5,0.5,0.5,1)
    mesh.materials.append(material)
    bpy.context.scene['frozen_baseline'] = False
    bpy.context.scene['frozen_import_parent'] = str(baseline / 'nottingham-baseline.blend')
    bpy.context.view_layer.update()
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(target))
    current = json.loads((OUT / 'inventory/inventory.json').read_text())
    shutil.copy2(OUT / 'inventory/inventory.json', OUT / 'inventory/imported-inventory.json')
    inventory(OUT / 'inventory/recovered-basic', collection_name=collection.name, map_name='nottingham',
              source_path=baseline / 'Nottingham.map.png')
    fresh = json.loads((OUT / 'inventory/recovered-basic/inventory.json').read_text())
    for record in current['objects']:
        if record['source_node'] == 'terrace-366':
            record['source_node'] = 'building-366'
            record['custom_properties']['source_node'] = 'building-366'
    record = next(r for r in fresh['objects'] if r['source_node'] == 'building-082')
    geometry = {'vertices': [list(v.co) for v in mesh.vertices], 'faces': [list(p.vertices) for p in mesh.polygons]}
    record.update(parent=None, source_obstacle='building-082', editor_name=obj.name,
                  matrix_world=[list(row) for row in obj.matrix_world], matrix_local=[list(row) for row in obj.matrix_local],
                  matrix_parent_inverse=[list(row) for row in obj.matrix_parent_inverse],
                  geometry=geometry, uv_layers={layer.name: [list(loop.uv) for loop in layer.data] for layer in mesh.uv_layers},
                  geometry_sha256=json_hash(geometry),
                  uv_sha256=json_hash({layer.name: [list(loop.uv) for loop in layer.data] for layer in mesh.uv_layers}),
                  materials=[material.name], polygon_material_indices=[0]*len(faces), modifiers=[],
                  custom_properties={key: obj[key] for key in obj.keys()}, animation_present=False,
                  mask_associations=[], state_associations=[], association_status='Requires native patch/state reconciliation',
                  recovery={'native_obstacle': obstacle, 'native_source_sha256': digest(source), 'inferred_geometry': False})
    current['objects'].append(record)
    current['materials'].append({'name': material.name, 'use_nodes': False, 'images': []})
    current['objects'].sort(key=lambda r: r['source_node'])
    current['source_blend'] = str(target)
    current['source_blend_sha256'] = digest(target)
    current['frozen_import'] = str(baseline / 'nottingham-baseline.blend')
    current['source_inputs'][source.name] = {'original_path': str(source), 'sha256': digest(source)}
    (OUT / 'inventory/inventory.json').write_text(json.dumps(current, indent=2)+'\n')
    recovery = {'source_blend': str(target), 'sha256': digest(target), 'native_source_sha256': digest(source),
                'recovered_nodes': ['building-082'], 'normalized_node_ids': {'terrace-366': 'building-366'},
                'geometry': geometry, 'script_sha256': digest(__file__), 'existing_geometry_changed': False}
    (baseline / 'recovery-manifest.json').write_text(json.dumps(recovery, indent=2)+'\n')
    print(json.dumps({'recovered_scene': str(target), 'mesh_count': len(current['objects'])}))


def annotate_states(path=None):
    path = Path(path) if path else OUT / 'inventory/inventory.json'
    data = json.loads(path.read_text())
    layer_path = OUT / 'source-states/layers.json'
    layers = json.loads(layer_path.read_text())
    if layers['map'].casefold() != 'nottingham':
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
    if not any(material['name'] == 'Recovered source geometry - unknown neutral' for material in data['materials']):
        data['materials'].append({'name': 'Recovered source geometry - unknown neutral', 'use_nodes': False, 'images': []})
    data['patch_inventory_status'] = 'Native state obstacle membership supplied; visual receiver and mask ownership review required'
    path.write_text(json.dumps(data, indent=2)+'\n')
    original = json.loads((OUT / 'inventory/imported-inventory.json').read_text())
    previous = {obj['source_obstacle']: obj for obj in original['objects']}
    unchanged = all(all(obj[key] == previous[obj['source_obstacle']][key]
                        for key in ['geometry_sha256', 'uv_sha256', 'matrix_world', 'matrix_local', 'matrix_parent_inverse'])
                    for obj in data['objects'] if obj['source_node'] != 'building-082')
    coverage = {obj['source_node'] for obj in data['objects']} == {f'building-{i:03}' for i in range(555)} | {'ground'}
    frozen = json.loads((OUT / 'baseline/manifest.json').read_text())
    intact = digest(OUT / 'baseline/nottingham-baseline.blend') == frozen['inputs']['nottingham-baseline.blend']['sha256']
    validation = {'status': 'PASS' if unchanged and coverage and intact else 'FAIL',
                  'existing_geometry_uv_transforms_unchanged': unchanged, 'all_native_obstacles_present': coverage,
                  'frozen_import_hash_unchanged': intact, 'recovered_nodes': ['building-082'],
                  'state_associated_nodes': len(associations), 'mesh_count': len(data['objects'])}
    (OUT / 'inventory/setup-validation.json').write_text(json.dumps(validation, indent=2)+'\n')
    if validation['status'] != 'PASS':
        raise ValueError(validation)
    shutil.copy2(__file__, OUT / 'baseline/setup_scene.py')
    print(json.dumps(validation))


def repair_recovered_material():
    import bpy
    source = OUT / 'baseline/nottingham-recovered.blend'
    target = OUT / 'baseline/nottingham-recovered-v2.blend'
    if target.exists():
        raise FileExistsError(target)
    bpy.ops.wm.open_mainfile(filepath=str(source))
    bpy.context.window.scene = bpy.data.scenes['nottingham Refinement']
    obj = next(obj for obj in bpy.data.collections['nottingham Working'].objects if obj.get('source_node') == 'building-082')
    recovered_uv(obj.data)
    bpy.ops.wm.save_as_mainfile(filepath=str(target))
    original_inventory = OUT / 'inventory/inventory.json'
    path = OUT / 'inventory/inventory-v2.json'
    if path.exists():
        raise FileExistsError(path)
    data = json.loads(original_inventory.read_text())
    record = next(obj for obj in data['objects'] if obj['source_node'] == 'building-082')
    uvs = {layer.name: [list(loop.uv) for loop in layer.data] for layer in obj.data.uv_layers}
    record['uv_layers'] = uvs
    record['uv_sha256'] = json_hash(uvs)
    data['source_blend'] = str(target)
    data['source_blend_sha256'] = digest(target)
    path.write_text(json.dumps(data, indent=2)+'\n')
    (OUT / 'baseline/uv-repair-manifest.json').write_text(json.dumps({
        'source': str(source), 'source_sha256': digest(source), 'output': str(target),
        'output_sha256': digest(target), 'changed_node': 'building-082',
        'change': 'Add fallback source projection UVs to recovered neutral receiver',
        'geometry_unchanged': True, 'uv_sha256': record['uv_sha256'],
        'script_sha256': digest(__file__)}, indent=2)+'\n')
    annotate_states(path)


if __name__ == '__main__':
    if '--illustrate' in sys.argv:
        illustrate()
    elif '--recover' in sys.argv:
        recover()
    elif '--annotate-states' in sys.argv:
        annotate_states()
    elif '--repair-recovered-material' in sys.argv:
        repair_recovered_material()
    else:
        setup()
