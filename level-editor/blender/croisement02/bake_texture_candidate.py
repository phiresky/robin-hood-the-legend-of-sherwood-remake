"""Bake reviewed Croisement02 fills through the shared guarded texture stage.

Default mode is read-only preflight. A real bake additionally requires a
hash-bound manual review receipt for the raw and protected generation images.
No approved worker is modified and no result is published.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from render_slots import acquire, release
from review_evidence import sha
from prepare_texture_packet import prepare
from refinement_workspace import _geometry
from workspace_components import appearance_state
from bake_reviewed_asset import stage


def read(path):
    return json.loads(Path(path).read_text())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def array_hash(value):
    return hashlib.sha256(np.asarray(value).tobytes()).hexdigest()


def pixels(image):
    values = np.empty(len(image.pixels), dtype=np.float32)
    image.pixels.foreach_get(values)
    return values.reshape(image.size[1], image.size[0], 4)


def snapshot(scene, receiver_names):
    cache = {}
    result = {'geometry': {obj.name: _geometry(obj) for obj in scene.objects},
              'outside_appearance': {obj.name: digest(appearance_state(obj, cache))
                  for obj in scene.objects if obj.type == 'MESH' and obj.name not in receiver_names},
              'physical_foliage': {}}
    for name in sorted(receiver_names):
        obj = scene.objects[name]
        mesh = obj.data
        ownership = mesh.color_attributes.get('Source ownership')
        for slot, material in enumerate(mesh.materials):
            if not material or not material.get('foliage_physical_opacity'):
                continue
            require(ownership is not None and ownership.domain == 'CORNER', 'Missing physical foliage ownership')
            for key, value in {'opacity_semantics': 'physical-coverage',
                               'source_ownership_semantics': 'separate-mask',
                               'source_ownership_channel': 'vertex-color-r'}.items():
                require(material.get(key) == value, 'Invalid foliage contract: ' + material.name)
            faces = [face for face in mesh.polygons if face.material_index == slot]
            flags = [[ownership.data[i].color[0] for i in face.loop_indices] for face in faces]
            require(all(set(values) in ({0.}, {1.}) for values in flags), 'Mixed ownership within a foliage face')
            # Retained unused physical slots have no editable faces. Freeze their
            # complete atlas as well, without deleting approved material metadata.
            known = {values[0] for values in flags} if faces else {1.}
            require(len(known) == 1, 'Atlas mixes known and unknown faces')
            textures = [n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
            require(len(textures) == 1, 'Expected one physical foliage atlas')
            texture = textures[0]
            links = texture.inputs['Vector'].links
            require(len(links) == 1 and links[0].from_node.type == 'UVMAP', 'Expected explicit physical UV')
            uv_name = links[0].from_node.uv_map
            uv = mesh.uv_layers[uv_name]
            rgba = pixels(texture.image)
            result['physical_foliage'][name + '/' + str(slot)] = {
                'faces': [face.index for face in faces],
                'ownership': flags,
                'uv_name': uv_name,
                'uv': array_hash(np.array([uv.data[i].uv[:] for face in faces for i in face.loop_indices], dtype=np.float32)),
                'image_size': list(texture.image.size),
                'alpha': array_hash(rgba[:, :, 3]),
                'known_rgba': array_hash(rgba) if known == {1.} else None,
                'backface_culling': material.use_backface_culling,
                'opacity_cutoff': material.get('foliage_alpha_cutoff'),
            }
    return result


def preflight(experiment):
    approval = read(experiment / 'approval.json')
    manifest = read(experiment / 'views.json')
    preparation = read(experiment / 'preparation.json')
    source = Path(preparation['source_review_manifest'])
    require(sha(source) == preparation['review_manifest_sha256'], 'Preparation manifest changed')
    prepare(source, manifest['asset_id'], experiment / 'preflight-check-only', source.parent / 'decisions.json', check_only=True)
    for relative, expected in preparation['files'].items():
        require(sha(experiment / relative) == expected, 'Prepared artifact changed: ' + relative)
    require(sha(experiment / 'approved-model.blend') == approval['saved_model_sha256'], 'Approved model changed')
    bpy.ops.wm.open_mainfile(filepath=str(experiment / 'approved-model.blend'))
    scene = bpy.data.scenes[manifest['scene_name']]
    bpy.context.window.scene = scene
    names = set(manifest.get('texture_receiver_object_names', manifest['object_names']))
    require(names and all(name in scene.objects for name in names), 'Missing target mesh')
    require(all(scene.objects[name].type == 'MESH' and scene.objects[name].get('asset_group') == manifest['asset_id'] for name in names), 'Receiver selection includes foreign geometry')
    state = snapshot(scene, names)
    report = {'asset_id': manifest['asset_id'], 'status': 'PASS', 'mode': 'preflight-only',
              'model_sha256': approval['saved_model_sha256'], 'input_sha256': sha(experiment / 'input.png'),
              'receiver_names': sorted(names), 'physical_foliage_atlases': len(state['physical_foliage']),
              'known_physical_atlases': sum(value['known_rgba'] is not None for value in state['physical_foliage'].values()),
              'outside_meshes': len(state['outside_appearance']), 'snapshot_sha256': digest(state)}
    return manifest, scene, names, state, report


def run(experiment, output=None, review_path=None, texels_per_unit=2.):
    experiment = experiment.resolve(strict=True)
    acquire()
    try:
        manifest, scene, names, before, report = preflight(experiment)
        if output is None:
            print(json.dumps(report), flush=True)
            return report
        require(review_path is not None, 'A manual generation review receipt is required')
        review = read(review_path)
        require(review.get('ready_for_bake') is True and bool(review.get('reviewer')), 'Generation has not passed manual review')
        require(review['asset_id'] == manifest['asset_id'], 'Review belongs to another asset')
        raw, generated = Path(review['raw_image']), Path(review['preserved_image'])
        for path, expected in [(raw, review['raw_sha256']), (generated, review['preserved_sha256']),
                               (experiment / 'input.png', review['input_sha256']),
                               (experiment / 'mask.png', review['mask_sha256']),
                               (experiment / 'approved-model.blend', review['model_sha256'])]:
            require(sha(path) == expected, 'Generation review became stale: ' + str(path))
        source = np.asarray(Image.open(experiment / 'input.png').convert('RGBA'))
        preserved = np.asarray(Image.open(generated).convert('RGBA'))
        mask = np.asarray(Image.open(experiment / 'mask.png').convert('RGBA'))[:, :, 3] == 0
        require(source.shape == preserved.shape, 'Protected image dimensions changed')
        require(np.array_equal(source[~mask], preserved[~mask]), 'Protected known or background pixels changed')
        require(np.array_equal(source[:, :, 3], preserved[:, :, 3]), 'Protected image alpha changed')
        require(not output.exists(), 'Bake output already exists')
        raw_content = raw
        # Transport padding is removed by an exact crop, never scaling, solely
        # for tone reconciliation against the original frozen content canvas.
        raw_size = Image.open(raw).size
        size = (source.shape[1], source.shape[0])
        if raw_size != size:
            padding = manifest.get('transport_padding')
            require(padding and raw_size == (padding['width'], padding['height']), 'Unexpected raw dimensions')
            box = padding['content_box']
            require((box['width'], box['height']) == size, 'Transport content box changed')
            raw_content = output.parent / (output.name + '-raw-content.png')
            require(not raw_content.exists(), 'Reconciliation crop already exists')
            Image.open(raw).crop((box['left'], box['top'], box['left'] + box['width'], box['top'] + box['height'])).save(raw_content)
        evidence = {str(path): sha(path) for path in [review_path, raw, generated, experiment / 'views.json', experiment / 'approved-model.blend']}
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.transparent_max_bounces = 64
        staged = stage(experiment / 'views.json', generated, output,
                       texels_per_unit=texels_per_unit, reconciliation_reference=raw_content)
        require(snapshot(scene, names) == before, 'Bake changed geometry, foreign appearance, physical alpha, known foliage, or foliage UV/ownership')
        model = output / 'worker.blend'
        model_hash = sha(model)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        reopened = bpy.data.scenes[manifest['scene_name']]
        require(snapshot(reopened, names) == before, 'Saved candidate failed reopened preservation')
        require(all(sha(Path(path)) == expected for path, expected in evidence.items()), 'Evidence changed during bake')
        result = dict(report, mode='baked-candidate', candidate_model_sha256=model_hash,
                      generation_review=str(review_path), generation_review_sha256=sha(review_path),
                      evidence_sha256=evidence, geometry_unchanged=True,
                      foreign_appearance_unchanged=True, physical_alpha_unchanged=True,
                      known_foliage_rgba_unchanged=True, foliage_uv_and_ownership_unchanged=True,
                      reopened_preservation='PASS', actual_material_review='pending',
                      texture_approval='pending', publication='not performed',
                      bake_validation_sha256=sha(output / 'validation.json'),
                      physical_foliage=staged.get('physical_foliage', []))
        (output / 'reopened-preservation.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result), flush=True)
        return result
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiment', type=Path)
    parser.add_argument('--output', type=Path, help='Omit for read-only preflight')
    parser.add_argument('--review', type=Path, help='Manual raw/protected generation review receipt')
    parser.add_argument('--texels-per-unit', type=float, default=2.)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    run(args.experiment, args.output, args.review, args.texels_per_unit)
