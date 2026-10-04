"""Restore explicitly reviewed mixed-mask ground pixels in a scoped derivative."""
import json
import argparse
from array import array
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json, digest
from render_slots import acquire, release
from prepare_ground_receiver import material
from render_multiview_asset import render
from refinement_review import _tile


def geometry_data(obj):
    return dict(vertices=[list(v.co) for v in obj.data.vertices],
                       faces=[list(p.vertices) for p in obj.data.polygons],
                       uv=[[list(v.uv) for v in layer.data] for layer in obj.data.uv_layers],
                       matrix=[list(r) for r in obj.matrix_world])


def geometry(obj):
    return digest(geometry_data(obj))


def main():
    baseline = OUT / 'ground-receiver-review-v5'
    authority = OUT / 'understory-candidates/mixed75-91-source-v3'
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=OUT / 'ground-source75-restoration-v1')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
    output = args.output.resolve()
    if output.exists():
        raise FileExistsError(output)
    proof_path = authority / 'ground-return75.json'
    proof = json.loads(proof_path.read_text())
    domain_path = Path(proof['aggregate'])
    if sha(domain_path) != proof['sha256']:
        raise ValueError('Returned ground authority changed')
    for row in proof['inputs']:
        if sha(Path(row['path'])) != row['sha256']:
            raise ValueError('Returned ground input changed')
    if sha(authority / 'domain-487.png') != proof['leaf_domain_sha256']:
        raise ValueError('Leaf75 authority changed')
    returned = np.asarray(Image.open(domain_path).convert('L')) > 0
    leaf = np.asarray(Image.open(authority / 'domain-487.png').convert('L')) > 0
    reference = baseline / 'reference'
    known = np.asarray(Image.open(reference / 'ground-observed-domain.png').convert('L')) > 0
    plane = np.asarray(Image.open(reference / 'ground-first-hit.png').convert('L')) > 0
    source = np.asarray(Image.open(reference / 'source.png').convert('RGB'))
    old = np.asarray(Image.open(reference / 'observed-neutral.png').convert('RGB'))
    if int(returned.sum()) != 783 or (returned & leaf).any() or (returned & ~plane).any():
        raise ValueError('Returned ground does not match exact reviewed receiver scope')
    if (returned & known).any():
        raise ValueError('Expected previously reserved ground only')
    for name in ['authored-scenery-reservation', 'animated-first-frame-exclusion']:
        foreign = np.asarray(Image.open(reference / f'{name}.png').convert('L')) > 0
        if (foreign & returned).any():
            raise ValueError('Ground restoration overlaps a foreign owner: ' + name)
    updated = old.copy()
    updated[returned] = source[returned]
    if not np.array_equal(updated[~returned], old[~returned]):
        raise ValueError('Changed pixels outside explicit ground restoration')
    model_hash = sha(baseline / 'model.blend')
    acquire()
    try:
        output.mkdir()
        Image.fromarray(updated).save(output / 'observed-neutral.png')
        Image.fromarray((known | returned).astype('uint8') * 255).save(output / 'ground-observed-domain.png')
        bpy.ops.wm.read_factory_settings(use_empty=True)
        scene = bpy.context.scene
        scene.name = 'Croisement02 Refinement'
        with bpy.data.libraries.load(str(baseline / 'model.blend'), link=False) as (src, dst):
            if 'Croisement02 Terrain' not in src.objects:
                raise ValueError('Frozen ground object missing')
            dst.objects = ['Croisement02 Terrain']
        obj = dst.objects[0]
        scene.collection.objects.link(obj)
        parent = obj.parent
        while parent is not None:
            if parent.name not in scene.objects:
                scene.collection.objects.link(parent)
            parent = parent.parent
        bpy.context.view_layer.update()
        # Retain the exact source hierarchy. Matrix assignment after detaching
        # can decompose a nearly right-angle rotation with measurable drift.
        source_geometry = {key:value for key,value in geometry_data(obj).items() if key != 'uv'}
        expected_geometry = json.loads((baseline / 'validation.json').read_text())['geometry_signature']
        if digest(source_geometry) != expected_geometry:
            raise ValueError('Imported ground differs from frozen source geometry/transform')
        before = geometry(obj)
        write_json(output / 'geometry-before.json', geometry_data(obj))
        material(obj, output / 'observed-neutral.png')
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.view_settings.view_transform = 'Standard'
        scene.render.film_transparent = True
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'), compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(output / 'model.blend'))
        obj = bpy.data.objects['Croisement02 Terrain']
        bpy.context.view_layer.update()
        write_json(output / 'geometry-reopened.json', geometry_data(obj))
        if geometry(obj) != before:
            raise ValueError('Saved ground geometry or UV changed')
        tex = next(n for n in obj.data.materials[0].node_tree.nodes if n.type == 'TEX_IMAGE')
        pixels = np.empty(len(tex.image.pixels), np.float32)
        tex.image.pixels.foreach_get(pixels)
        packed = np.rint(pixels.reshape(1152, 1792, 4)[::-1, :, :3] * 255).astype('uint8')
        if not np.array_equal(packed, updated):
            raise ValueError('Reopened packed ground RGB differs from prepared atlas')
        manifest = json.loads((baseline / 'input/views.json').read_text())
        for view in manifest['views']:
            view['crop'] = dict(width=512, height=384)
        write_json(output / 'views.json', manifest)
        render(output / 'views.json', output / 'actual', width=512)
        buffers = []
        for index in range(8):
            im = bpy.data.images.load(str(output / 'actual' / f'view-{index}-textured.png'))
            values = array('f', [0]) * len(im.pixels)
            im.pixels.foreach_get(values)
            buffers.append(values)
        _tile(buffers, 512, 384, output / 'actual/textured.png')
        if sha(baseline / 'model.blend') != model_hash:
            raise ValueError('Frozen ground model changed')
        write_json(output / 'validation.json', dict(status='PASS', source_model_sha256=model_hash,
            model_sha256=sha(output / 'model.blend'), geometry_uv_signature=before,
            frozen_geometry_signature=expected_geometry,
            geometry_uv_unchanged=True, restored_source_pixels=783,
            existing_known_pixels_unchanged=True, all_other_rgb_unchanged=True,
            foliage75_overlap=0, authored_or_animated_foreign_overlap=0,
            authority=str(proof_path), authority_sha256=sha(proof_path),
            known_pixels=int((known | returned).sum()), packed_atlas_exact=True,
            scope='Ground75 role restoration only. Final whole-scene foreground audit remains required.',
            user_approval='pending', integration='not performed'))
    finally:
        release()


if __name__ == '__main__':
    main()
