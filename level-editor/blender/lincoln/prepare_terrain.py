"""Create the Lincoln terrain workspace (asset lincoln-terrain, source node ground).

Ground is the canonical terrain receiver outside the building catalog, so the
catalog-driven ``prepare()`` cannot create it. This recipe follows the shared
workspace contract (frozen baseline, input/ and modified/ eight-view packets,
frozen mask reference, outside-geometry hashes, ``validate()``) and the
Nottingham terrain precedent:

1. Author the reviewed ground receiver domain (terrain_ground_domain.py) from
   the source-camera first hit of the refined ground in the integrated scene.
2. Reopen the frozen scene, bind the ground to ``lincoln-terrain`` and freeze
   the baseline, a mask inventory (v4 plus the authored ground domain) and the
   working source-masks.json with the ground assignment.
3. Render the frozen input packet (flat ground), apply terrain_ground.build()
   (stream channel), store the observed-source ground material, save
   model.blend and render the modified packet with the same cameras.

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/prepare_terrain.py -- \
        [--source-blend <scene.blend>] [--output <workspace>]
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
from mathutils import Vector

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
sys.path.insert(0, str(HERE))
from render_slots import acquire  # noqa: E402
from freeze_tooling import select_tooling  # noqa: E402
import terrain_ground  # noqa: E402
import terrain_ground_domain  # noqa: E402

ASSET = 'lincoln-terrain'
SCENE = 'lincoln Refinement'
COLLECTION = 'lincoln Working'
TILE = (512, 384)
CONTEXT_CROP = {'left': 0, 'top': 0, 'right': 1450, 'bottom': 1150}
MASKS = ROOT / 'mask-review/source-masks-v4.json'
SOURCE = ROOT / 'source-states/covered.png'
LAYERS = ROOT / 'source-states/layers.json'
LIGHTING = ROOT / 'lighting-calibration/map-lighting.json'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def working_meshes():
    return [o for o in bpy.data.collections[COLLECTION].all_objects if o.type == 'MESH']


def the_ground():
    grounds = [o for o in working_meshes() if o.get('source_node') == 'ground']
    if len(grounds) != 1 or grounds[0].hide_render:
        raise ValueError(f'Expected one visible ground receiver, got {[o.name for o in grounds]}')
    return grounds[0]


def first_hit():
    """Source-camera z-buffer of every render-visible working mesh (label, names)."""
    import numpy as np
    import rocks_terrain_volumes_core as core
    depth = np.full((2176, 2944), -np.inf)
    label = np.zeros((2176, 2944), np.int32)
    names = ['']
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in working_meshes():
        if obj.hide_render:
            continue
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        mesh.calc_loop_triangles()
        mw = obj.matrix_world
        co = np.array([list(mw @ v.co) for v in mesh.vertices])
        idx = np.array([list(t.vertices) for t in mesh.loop_triangles], int).reshape(-1, 3)
        evaluated.to_mesh_clear()
        if not len(idx):
            continue
        names.append(obj.get('source_node') or obj.name)
        core.zbuffer(depth, label, co[idx], len(names) - 1, 0, 0)
    return label, names


def mask_inventory(output, domain):
    """V4 inventory with absolute bitmap paths plus the authored ground domain."""
    base = json.loads(MASKS.read_text())
    inventory_path = (MASKS.parent / base['mask_inventory']).resolve(strict=True)
    inventory = json.loads(inventory_path.read_text())
    for record in inventory['masks']:
        if record.get('png') is not None:
            record['png'] = str((inventory_path.parent / record['png']).resolve(strict=True))
    index = max(r['index'] for r in inventory['masks']) + 1
    bitmap = output / domain['bitmap']['png']
    if sha(bitmap) != domain['bitmap']['sha256']:
        raise ValueError('Ground domain bitmap changed after authoring')
    inventory['masks'].append({
        'index': index, 'layer': None, 'layer_index': None, 'png': str(bitmap), 'mask_type': None,
        'box_top_left': domain['bitmap']['box_top_left'], 'box_size': domain['bitmap']['box_size'],
        'character_polyline': [], 'projectile_polyline': [], 'obstacle_indices': [], 'synthetic': True,
        'constraint_kind': 'reviewed-authored-receiver-domain', 'source_sha256': domain['source_sha256'],
        'review_evidence': str(output / 'inspection/ground-domain-ground.json'), 'pixels': domain['bitmap']['pixels'],
        'reason': 'Authored painted village/outer ground, stream water and banks; native masks outline scenery only.'})
    folder = output / 'source-domain'
    folder.mkdir()
    (folder / 'manifest.json').write_text(json.dumps(inventory, indent=2) + '\n')
    masks = json.loads(MASKS.read_text())
    masks['mask_inventory'] = str((folder / 'manifest.json').resolve())
    assignments = masks['projections']['exterior']['assignments']
    if any(row.get('source_node') == 'ground' for row in assignments):
        raise ValueError('Mask manifest already assigns ground')
    assignments.append({'reviewed': True, 'source_node': 'ground', 'mask_indices': [index],
                        'constraint_kind': 'reviewed-authored-receiver-domain', 'native_ownership_reviewed': True,
                        'review_note': (f'Authored ground domain {index} (inspection/ground-domain-ground.json): '
                                        'refined-ground first hit minus every covered-state native mask, other '
                                        'authored domains and inspected prop carve-outs.')})
    masks['limitations'] = masks.get('limitations', []) + [
        f'Terrain workspace working manifest: v4 plus the authored ground domain mask {index}; fold into v5.']
    path = output / 'source-masks.json'
    path.write_text(json.dumps(masks, indent=2, sort_keys=True) + '\n')
    return path, index


def install_ground_source_material(ground, source_path, mask_path, output):
    """Store the observed-source ground atlas (UV = source pixel) without draping scenery."""
    from PIL import Image, ImageChops
    masks = json.loads(Path(mask_path).read_text())
    inventory_path = Path(mask_path).parent / masks['mask_inventory']
    inventory = json.loads(inventory_path.read_text())
    entries = {row['index']: row for row in inventory['masks']}
    assignment, = [row for row in masks['projections']['exterior']['assignments'] if row.get('source_node') == 'ground']
    source = Image.open(source_path).convert('RGB')

    def layer(index):
        row = entries[index]
        png = Path(row['png'])
        png = png if png.is_absolute() else inventory_path.parent / png
        image = Image.new('L', source.size)
        image.paste(Image.open(png).convert('L').point(lambda v: 255 if v else 0), tuple(row['box_top_left']))
        return image
    known = Image.new('L', source.size)
    for index in assignment['mask_indices']:
        known = ImageChops.lighter(known, layer(index))
    # Reviewed exclusions (e.g. approved foliage asset domains) leave the observed atlas.
    for index in assignment.get('exclude_mask_indices', []):
        known = ImageChops.subtract(known, layer(index))
    atlas = Image.new('RGB', source.size, (128, 128, 128))
    atlas.paste(source, mask=known)
    atlas.putalpha(known)
    folder = output / 'projection'
    folder.mkdir(exist_ok=True)
    path = folder / 'ground-source-owned.png'
    atlas.save(path)
    image = bpy.data.images.load(str(path), check_existing=False)
    # Ownership alpha is data, not transparency.
    image.alpha_mode = 'CHANNEL_PACKED'
    image.reload()
    image.pack()
    material = bpy.data.materials.new('Terrain / reviewed ground source ownership')
    material['source_ownership_bake'] = True
    material['source_ownership_label'] = 'exterior'
    material['source_ownership_fill'] = 'neutral'
    material['source_ownership_alpha'] = 'one=observed,zero=inferred;material remains opaque'
    material['projection_preserve'] = True
    material['reprojection_source_sha256'] = sha(source_path)
    nodes, links = material.node_tree.nodes, material.node_tree.links
    nodes.clear()
    uv = nodes.new('ShaderNodeUVMap')
    uv.uv_map = ground.data.uv_layers.active.name
    texture = nodes.new('ShaderNodeTexImage')
    texture.image = image
    texture.interpolation = 'Closest'
    surface = nodes.new('ShaderNodeOutputMaterial')
    links.new(uv.outputs['UV'], texture.inputs['Vector'])
    links.new(texture.outputs['Color'], surface.inputs['Surface'])
    ground.data.materials.append(material)
    for face in ground.data.polygons:
        face.material_index = len(ground.data.materials) - 1
    report = {'status': 'PASS', 'source_sha256': sha(source_path), 'stored_atlas_sha256': sha(path),
              'receiver': ground.name, 'uv_layer': uv.uv_map,
              'uv': 'source pixel projection (x, y - z) of each vertex',
              'ownership_alpha': 'observed source only; opacity remains opaque',
              'known_source_pixels': sum(1 for p in known.getdata() if p > 127)}
    (folder / 'ground-material.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def geometry_hash(obj):
    return terrain_ground.digest({'vertices': [list(v.co) for v in obj.data.vertices],
                                  'faces': [list(p.vertices) for p in obj.data.polygons],
                                  'matrix': [list(r) for r in obj.matrix_world]})


def framing(image, lighting):
    """Eight fixed azimuth cameras around the complete stream channel (before and after)."""
    from setup_map import fit_camera
    from review_sunlight import configuration
    scene = bpy.data.scenes[SCENE]
    regions = terrain_ground.bed_regions()
    reach = -terrain_ground.BED_Z / terrain_ground.BANK_SLOPE
    points = []
    for region in regions:
        xs = [p[0] for p in region]
        ys = [p[1] for p in region]
        for x in (max(0.0, min(xs) - reach), min(2944.0, max(xs) + reach)):
            for y in (max(terrain_ground.TONGUE[2], min(ys) - reach), min(2176.0, max(ys) + reach)):
                for z in (0.0, terrain_ground.BED_Z):
                    points.append(Vector(terrain_ground.world((x, y, z))))
    target = sum(points, Vector((0, 0, 0))) / len(points)
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    views = []
    for index in range(8):
        data = bpy.data.cameras.new('Terrain frozen framing')
        camera = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(camera)
        yaw = math.radians(index * 45)
        camera.location = target + Vector((math.sin(yaw) * cosine, -math.cos(yaw) * cosine, sine)) * 10000
        camera.rotation_euler = (target - camera.location).to_track_quat('-Z', 'Y').to_euler()
        data.type = 'ORTHO'
        data.clip_end = 100000
        fit_camera(camera, [], TILE[0] / TILE[1], points=points, padding=1.04)
        views.append({'index': index, 'camera_location': list(camera.location),
                      'camera_rotation_euler': list(camera.rotation_euler), 'ortho_scale': data.ortho_scale})
        bpy.data.objects.remove(camera, do_unlink=True)
        bpy.data.cameras.remove(data)
    nodes = sorted({o.get('source_node') for o in working_meshes() if not o.hide_render}, key=str)
    layer = {'source_path': str(image), 'projection_label': 'exterior', 'receiver_nodes': nodes,
             'occluder_nodes': nodes, 'source_sha256': sha(image)}
    return {'version': 1, 'asset_id': ASSET, 'tile_size': list(TILE), 'elevation_degrees': 35,
            'source_sha256': sha(image), 'views': views,
            'lighting': configuration(lighting, map_name='lincoln'),
            'context_crop': CONTEXT_CROP, 'projection_layers': [layer],
            'framing': 'Complete stream channel footprint and banks (flat and cut), all eight azimuths'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-blend', type=Path, default=ROOT / 'grouped/lincoln-grouped-v4.blend')
    parser.add_argument('--output', type=Path, default=ROOT / 'round-4/assets' / ASSET)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else [])
    acquire()
    tooling = select_tooling(ROOT / 'tooling/e6b57cb851c7142b')
    from refinement_workspace import _ownership, _files, _freeze_masks, validate
    from refinement_review import render_review
    source_blend = args.source_blend.resolve(strict=True)
    source_hash = sha(source_blend)
    output = args.output.resolve()
    if output.exists():
        raise FileExistsError(output)
    output.mkdir(parents=True)
    reference = output / 'reference'
    reference.mkdir()
    image = reference / 'source.png'
    shutil.copy2(SOURCE, image)
    shutil.copy2(ROOT / 'grouping/catalog-v4.json', reference / 'catalog.json')
    shutil.copy2(LAYERS, reference / 'layers.json')
    lighting = json.loads(LIGHTING.read_text())['lighting']

    # 1. Author the ground domain against the refined ground in the integrated scene.
    bpy.ops.wm.open_mainfile(filepath=str(source_blend))
    bpy.context.window.scene = bpy.data.scenes[SCENE]
    terrain_ground.build(the_ground())
    label, names = first_hit()
    inv_path = (MASKS.parent / json.loads(MASKS.read_text())['mask_inventory']).resolve()
    domain = terrain_ground_domain.author(
        label, names, source=image, inventory_path=inv_path, layers_path=LAYERS,
        out_dir=output / 'inspection',
        scene_record={'source_blend': str(source_blend), 'source_blend_sha256': source_hash,
                      'geometry': 'terrain_ground.build() applied to the source blend ground',
                      'recipe_sha256': sha(terrain_ground.__file__)})
    mask_path, mask_index = mask_inventory(output, domain)
    print('Ground domain', domain['statistics'], flush=True)

    # 2. Fresh frozen scene: bind ownership and freeze the baseline.
    bpy.ops.wm.open_mainfile(filepath=str(source_blend))
    scene = bpy.data.scenes[SCENE]
    bpy.context.window.scene = scene
    ground = the_ground()
    ground['asset_group'] = ASSET
    ground['asset_name'] = 'Lincoln terrain'
    config = {'version': 1, 'asset_id': ASSET, 'scene_name': SCENE, 'collection_name': COLLECTION,
              'map_name': 'lincoln', 'part_ids': ['ground'], 'source_blend': str(source_blend),
              'source_blend_sha256': source_hash, 'source_path': str(image),
              'source_mask_manifest': str(mask_path), 'projection_manifest': None,
              'width': TILE[0], 'height': TILE[1], 'elevation_degrees': 35, 'context_padding': 24,
              'lighting': lighting,
              'terrain_role': 'Separate canonical terrain receiver; outside the building catalog',
              'tooling': tooling, 'source_mask_parent_manifest': str(MASKS), 'source_mask_parent_sha256': sha(MASKS),
              'authored_ground_mask_index': mask_index}
    _, config['outside_geometry'] = _ownership(config)
    _freeze_masks(output, config)
    outside = {o.name: geometry_hash(o) for o in working_meshes() if o != ground}
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'baseline.blend'), copy=True)
    config['baseline_sha256'] = sha(output / 'baseline.blend')
    frames = framing(image, lighting)
    (output / 'framing.json').write_text(json.dumps(frames, indent=2) + '\n')
    options = dict(scene_name=SCENE, collection_name=COLLECTION, asset_id=ASSET, source_path=image,
                   projection_layers=[{k: v for k, v in frames['projection_layers'][0].items() if k != 'source_sha256'}],
                   source_mask_manifest=mask_path)
    print('Rendering input packet', flush=True)
    baseline = render_review(output / 'input', frame_manifest=frames, **options)
    config['input_files'] = _files(output / 'input')
    config['reference_files'] = _files(reference)
    (output / 'workspace.json').write_text(json.dumps(config, indent=2) + '\n')

    # 3. Refine, store the observed-source material, save and render modified.
    report = terrain_ground.build(ground)
    bridge = [o for o in working_meshes() if o.get('source_node') in ('building-045', 'building-046')]
    report['footbridge_clearance'] = terrain_ground.footbridge_clearance(ground, bridge)
    material = install_ground_source_material(ground, image, mask_path, output)
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'))
    print('Rendering modified packet', flush=True)
    modified = render_review(output / 'modified', frame_manifest=output / 'input/views.json', **options)
    after = {o.name: geometry_hash(o) for o in working_meshes() if o != ground}
    if after != outside or sha(source_blend) != source_hash:
        raise ValueError('Terrain recipe changed outside geometry or the frozen source')
    for a, b in zip(baseline['views'], modified['views']):
        if any(a[key] != b[key] for key in ('camera_location', 'camera_rotation_euler', 'ortho_scale')):
            raise ValueError('Terrain comparison cameras differ')
    shared = validate(output)
    validation = {**shared, 'outside_objects_preserved': len(outside), 'source_blend_sha256': source_hash,
                  'baseline_sha256': sha(output / 'baseline.blend'), 'model_sha256': sha(output / 'model.blend'),
                  'source_image_sha256': sha(image), 'fixed_cameras_preserved': True, 'geometry': report,
                  'tooling': tooling, 'stored_ground_material': material,
                  'ownership': 'Explicit reviewed terrain source assignment; native exclusions and scene visibility retained',
                  'approval': 'pending', 'texture_generation': 'not started'}
    (output / 'geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    (output / 'validation.json').write_text(json.dumps(validation, indent=2) + '\n')
    binding = {**validation, 'version': 1, 'input_files': _files(output / 'input'),
               'reference_files': _files(reference), 'modified_files': _files(output / 'modified'),
               'workspace_sha256': sha(output / 'workspace.json'),
               'recipe_sha256': sha(terrain_ground.__file__), 'domain_script_sha256': sha(terrain_ground_domain.__file__),
               'packet_script_sha256': sha(__file__), 'argv': sys.argv,
               'accepted_known_pixels': sum(v['counts']['source'] for v in modified['views'])}
    (output / 'terrain-packet.json').write_text(json.dumps(binding, indent=2) + '\n')
    print(json.dumps({k: validation[k] for k in ('status', 'model_sha256', 'outside_objects_preserved')}))


if __name__ == '__main__':
    main()
