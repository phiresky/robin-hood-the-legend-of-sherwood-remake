"""Reset the published legacy worker to original-art-only texture evidence.

Run in Blender after install_editor_migration.py. The Day layer excludes the
separate authored canopy sprites. No old synthesized RGB is used as evidence.
Physical canopy alpha and source ownership are stored independently.
"""
import argparse
import hashlib
import json
import math
import shutil
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
EDITOR = HERE.parents[1]
sys.path[:0] = [str(HERE), str(EDITOR / 'refinement'),
                str(EDITOR / 'refinement/blender'), str(EDITOR / 'blender/lincoln')]
from paths import DATA
from render_slots import acquire
from source_projection_bake import bake
from stage_editor_migration import runtime_material
import global_reproject as gr

gr.SCENE = 'Sherwood Editor Migration'
gr.COLLECTION = 'Sherwood Working'
gr.OWNERSHIP_LABEL = 'exterior'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def geometry(objects):
    records = {}
    for obj in objects:
        records[obj.name] = {
            'vertices': [list(v.co) for v in obj.data.vertices],
            'faces': [list(p.vertices) for p in obj.data.polygons],
            'matrix': [list(row) for row in obj.matrix_world],
        }
    return hashlib.sha256(json.dumps(records, sort_keys=True).encode()).hexdigest()


def flag_physical(material):
    for key, value in {'foliage_physical_opacity': True,
                       'opacity_semantics': 'physical-coverage',
                       'source_ownership_semantics': 'separate-mask',
                       'source_ownership_channel': 'vertex-color-r',
                       'foliage_unlit': True}.items():
        material[key] = value


def main(stage, output, reuse_canopies=None):
    stage, output = Path(stage).resolve(), Path(output).resolve()
    if json.loads((stage / 'installation.json').read_text())['status'] != 'APPLIED':
        raise ValueError('Publish the normal editor migration before full reprojection')
    output.mkdir(parents=True, exist_ok=False)
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(stage / 'editor-migration.blend'))
    bpy.context.window.scene = bpy.data.scenes[gr.SCENE]
    # Detach while the comparison scenes still exist: retained glTF meshes
    # inherit their coordinate conversion from parents in those scenes.
    working = bpy.data.collections[gr.COLLECTION]
    objects = list(working.objects)
    bpy.context.view_layer.update()
    published_matrices = {obj.name: obj.matrix_world.copy() for obj in objects}
    for obj in objects:
        obj.parent = None
        obj.matrix_world = published_matrices[obj.name]
    bpy.context.view_layer.update()
    for obj in objects:
        if not np.allclose(np.array(obj.matrix_world), np.array(published_matrices[obj.name]), atol=1e-5, rtol=0):
            raise ValueError('Detaching comparison parent changed transform: '+obj.name)
    before = geometry(objects)
    for scene in list(bpy.data.scenes):
        if scene != bpy.context.scene:
            bpy.data.scenes.remove(scene)
    bpy.ops.outliner.orphans_purge(do_recursive=True)
    if geometry(objects) != before:
        raise ValueError('Removing comparison scenes changed published geometry')
    leaves = [obj for obj in objects if 'Static leaves' in obj.data.uv_layers]
    opaque = [obj for obj in objects if obj not in leaves]
    print(f'Reprojecting {len(opaque)} opaque meshes and {len(leaves)} canopy meshes', flush=True)
    source = DATA / 'Levels/Day/sherwood.map.png'
    evidence = output / 'ownership'
    evidence.mkdir()
    # Day contains bare branches and trunks, not the separate Arbre overlays.
    # Hide leaves for this layer so they do not erase valid Day evidence.
    for obj in leaves:
        obj.hide_render = True
    day_scene = gr.Scene()
    ids, _ = gr.rasterize(day_scene, 1920, 1088)
    ground_triangle = np.array([day_scene.objects[i].get('source_node') == 'ground'
                                for i in day_scene.tri_obj])
    blocked = (ids < 0) | ~ground_triangle[np.maximum(ids, 0)]
    ground_occluded = blocked.reshape(1088, gr.SS, 1920, gr.SS).any(axis=(1, 3))
    del day_scene, ids, blocked, ground_triangle
    print('Ground visibility complete; baking original Day artwork', flush=True)
    bake('Sherwood', source, output / 'day-reprojection.json',
         projection_label='exterior', preserve_authored=False,
         collection_name=gr.COLLECTION, provenance_directory=evidence)
    for obj in leaves:
        obj.hide_render = False
    ownership = {}
    report = json.loads((output / 'day-reprojection.json').read_text())
    for row in report['objects']:
        if 'texel_provenance' in row:
            ownership[row['object']] = row['texel_provenance']['path']

    if reuse_canopies:
        prior_path = Path(reuse_canopies).resolve()
        prior = json.loads((prior_path/'reprojection.json').read_text())
        if sha(prior_path/'source-only.blend') != prior['worker_sha256']:
            raise ValueError('Reusable canopy worker changed')
        names = [obj.name for obj in leaves]
        with bpy.data.libraries.load(str(prior_path/'source-only.blend'), link=False) as (available, loaded):
            loaded.objects = names
        for obj, previous in zip(leaves, loaded.objects):
            if previous is None:
                raise ValueError('Missing reusable canopy: '+obj.name)
            if (len(obj.data.vertices) != len(previous.data.vertices)
                    or len(obj.data.polygons) != len(previous.data.polygons)
                    or not np.allclose(np.array(obj.matrix_world), np.array(previous.matrix_world), atol=1e-5, rtol=0)
                    or any(a.co != b.co for a,b in zip(obj.data.vertices, previous.data.vertices))
                    or any(tuple(a.vertices) != tuple(b.vertices) for a,b in zip(obj.data.polygons, previous.data.polygons))):
                raise ValueError('Canopy geometry differs; cannot reuse projection: '+obj.name)
            old_mask = Path(prior['ownership'][obj.name])
            if sha(old_mask) != prior['ownership_sha256'][obj.name]:
                raise ValueError('Reusable canopy ownership changed')
            new_mask = evidence/old_mask.name
            shutil.copy2(old_mask, new_mask)
            ownership[obj.name] = str(new_mask)
            obj.data = previous.data
            bpy.data.objects.remove(previous, do_unlink=True)
        source_records = prior['canopy_sources']
        print('Reused verified original-art canopy projections and physical coverage', flush=True)
    else:
        # Retain authored cutout geometry as separate visibility proxies while the
        # receivers get new per-face UVs. The original worker remains frozen.
        proxies = []
        old_leaf_data = {}
        for obj in leaves:
            material = obj.data.materials[0]
            flag_physical(material)
            old_leaf_data[obj.name] = (material, gr.slot_uvs(obj, 'Static leaves').copy())
            proxy = obj.copy()
            proxy.data = obj.data.copy()
            working.objects.link(proxy)
            proxy['source_node'] = 'scenery-projection-proxy-' + obj.name
            proxies.append(proxy)
            for index, mat in enumerate(list(obj.data.materials)):
                mat = mat.copy()
                del mat['foliage_physical_opacity']
                obj.data.materials[index] = mat
        manifest_path = EDITOR / 'work/needs-review/sherwood-refinement/animation-references/manifest.json'
        records = json.loads(manifest_path.read_text())['assets']
        source_records = []
        for record in records:
            if record['kind'] != 'tree':
                continue
            profile = record['profile']
            receivers = [obj for obj in leaves if old_leaf_data[obj.name][0].name.startswith(profile)]
            if not receivers:
                raise ValueError('Missing canopy receivers: ' + profile)
            # The first-frame canvas was extracted directly from the original game.
            sprite = manifest_path.parent / record['first_png']
            image = Image.open(sprite).convert('RGBA')
            canvas = Image.new('RGBA', (max(1920, record['left'] + image.width),
                                        max(1088, record['top'] + image.height)))
            canvas.paste(image, (record['left'], record['top']))
            layer = output / (profile.rsplit(' - ', 1)[-1].lower() + '-source.png')
            canvas.save(layer)
            target_report = output / (layer.stem + '-reprojection.json')
            print(f'Baking original canopy {profile}: {len(receivers)} meshes', flush=True)
            bake('Sherwood', layer, target_report, projection_label='exterior',
                 preserve_authored=False, receiver_object_names=[o.name for o in receivers],
                 occluder_nodes=[p['source_node'] for p in proxies],
                 collection_name=gr.COLLECTION, provenance_directory=evidence)
            for row in json.loads(target_report.read_text())['objects']:
                if 'texel_provenance' in row:
                    ownership[row['object']] = row['texel_provenance']['path']
            source_records.append({'profile': profile, 'source': str(sprite), 'sha256': sha(sprite),
                                   'canvas': str(layer), 'canvas_sha256': sha(layer)})
        for proxy in proxies:
            bpy.data.objects.remove(proxy, do_unlink=True)

        scene = gr.Scene()
        for record in scene.meshes:
            obj = record['object']
            if obj not in leaves:
                continue
            # Resample the original alpha through the old UVs at each new atlas
            # texel. This retains holes even on source-unseen faces.
            material, old_uv = old_leaf_data[obj.name]
            old_image = next(n.image for n in material.node_tree.nodes if n.type == 'TEX_IMAGE')
            old_rgba = gr.read_image(old_image)
            binding = scene.slot_binding(obj, int(record['slots'][0]))
            image = binding['image']
            atlas = gr.read_image(image)
            source_alpha = atlas[..., 3].copy()
            physical = np.zeros(atlas.shape[:2], np.uint8)
            flags = np.load(ownership[obj.name])['ownership']
            rejected = (flags == 1) & (source_alpha < 128)
            flags[rejected] = 0
            colors = obj.data.color_attributes.get('source_ownership')
            if colors is None:
                colors = obj.data.color_attributes.new(name='source_ownership', type='FLOAT_COLOR', domain='CORNER')
            obj.data.color_attributes.active_color = colors
            uv = gr.slot_uvs(obj, binding['uv'])
            triangles_by_face = {}
            for triangle, face_index in enumerate(record['polygons']):
                triangles_by_face.setdefault(int(face_index), []).append(triangle)
            for face, rows, cols, positions, normals, interior in gr.islands(record, uv, image.size, lambda group: True):
                triangles = triangles_by_face[face]
                best = np.full(len(rows), -np.inf)
                mapped = np.zeros((len(rows), 2))
                for t in triangles:
                    a, b, c = record['corners'][t]
                    ab, ac, ap = b-a, c-a, positions-a
                    aa, bb, cc = ab@ab, ab@ac, ac@ac
                    determinant = aa*cc-bb*bb
                    if abs(determinant) < 1e-20:
                        continue
                    u = (cc*(ap@ab)-bb*(ap@ac))/determinant
                    v = (aa*(ap@ac)-bb*(ap@ab))/determinant
                    weights = np.stack([1-u-v, u, v], 1)
                    margin = weights.min(1)
                    take = margin > best
                    mapped[take] = weights[take] @ old_uv[record['loops'][t]]
                    best[take] = margin[take]
                height, width = old_rgba.shape[:2]
                x = np.clip(np.floor(mapped[:, 0]*width).astype(int), 0, width-1)
                y = np.clip(np.floor(mapped[:, 1]*height).astype(int), 0, height-1)
                physical[rows, cols] = old_rgba[y, x, 3]
                # Runtime COLOR_0 is a conservative per-face display mask. The
                # external atlas mask remains the exact texel-level authority.
                covered = interior & (physical[rows, cols] >= 128)
                known = bool(covered.any() and np.all(flags[rows[covered], cols[covered]] == 1))
                for loop in obj.data.polygons[face].loop_indices:
                    colors.data[loop].color = (float(known), 1, 1, 1)
            atlas[..., 3] = physical
            atlas[rejected, :3] = 128
            np.savez_compressed(ownership[obj.name], ownership=flags)
            gr.write_image(image, atlas)
            replacement = runtime_material(obj.name + ' source-only leaves', image, binding['uv'], True)
            flag_physical(replacement)
            for key in ('source_ownership_bake', 'source_ownership_label', 'source_ownership_fill'):
                replacement[key] = binding['material'][key]
            obj.data.materials[int(record['slots'][0])] = replacement
            print(f'Restored physical coverage: {obj.name}', flush=True)

    # Full reset of ground RGB; old inpainted ground is never a source.
    # The audited native cleanup mask also excludes painted object remnants
    # outside the reconstructed silhouettes.
    cleanup = EDITOR / 'work/needs-review/sherwood-refinement/ground-reprojection/cleanup-mask.png'
    mask = (np.asarray(Image.open(cleanup).convert('L')) > 0) | ground_occluded
    Image.fromarray(mask.astype(np.uint8)*255).save(output / 'ground-unknown.png')
    rgba = np.asarray(Image.open(source).convert('RGBA')).copy()
    rgba[mask, :3] = 128
    rgba[..., 3] = np.where(mask, 0, 255)
    ground = next(obj for obj in opaque if obj.get('source_node') == 'ground')
    image = bpy.data.images.new('Sherwood original Day ground', width=rgba.shape[1], height=rgba.shape[0], alpha=True)
    image.alpha_mode = 'CHANNEL_PACKED'
    gr.write_image(image, rgba[::-1])
    layer = ground.data.uv_layers.get('Reprojected ground') or ground.data.uv_layers.new(name='Reprojected ground')
    for loop in ground.data.loops:
        point = ground.matrix_world @ ground.data.vertices[loop.vertex_index].co
        layer.data[loop.index].uv = (point.x/rgba.shape[1], 1+(point.y*math.sin(math.radians(35))+point.z*math.cos(math.radians(35)))/rgba.shape[0])
    ground.data.materials.clear()
    ground.data.materials.append(runtime_material('Sherwood source-only ground', image, layer.name))
    for face in ground.data.polygons:
        face.material_index = 0
    ground_mask = evidence / 'ground.npz'
    np.savez_compressed(ground_mask, ownership=(~mask[::-1]).astype(np.uint8))
    ownership[ground.name] = str(ground_mask)
    if geometry(objects) != before:
        raise ValueError('Full reprojection changed geometry')
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'source-only.blend'))
    write(output / 'reprojection.json', {'status': 'SOURCE_ONLY', 'geometry_sha256': before,
          'baseline': str(stage / 'editor-migration.blend'), 'source_day': str(source),
          'source_day_sha256': sha(source), 'canopy_sources': source_records,
          'ground_cleanup_mask': str(cleanup), 'ground_cleanup_sha256': sha(cleanup),
          'ownership': ownership, 'generated_rgb_reused': False,
          'ownership_sha256': {name: sha(path) for name, path in ownership.items()},
          'geometry_changed': False, 'published_parent_transforms_preserved': True, 'worker_sha256': sha(output / 'source-only.blend')})
    print('SOURCE REPROJECTION COMPLETE', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--reuse-canopies')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    main(args.stage, args.output, args.reuse_canopies)
