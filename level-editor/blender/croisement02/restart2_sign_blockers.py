"""Attribute sign first-hit blockers to unchanged neighboring asset groups."""
import sys
import json
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, RAY
from restart2_sign_neighbors import camera_to, render
from stage_review_scene import signature


def body(image):
    a = np.asarray(image)[1::3, 1::3]
    return (a[:, :, 0] > 245) & (a[:, :, 1] < 10) & (a[:, :, 2] > 245) & (a[:, :, 3] > 127)


def main():
    base = OUT / 'restart2-fence/sign-neighbors-v3'
    evidence = json.loads((base / 'manifest.json').read_text())
    assembly = json.loads((OUT / 'state-sign-candidate/five-instances-v3/assembly.json').read_text())
    model = OUT / 'state-sign-candidate/five-instances-v3/model.blend'
    assert sha(model) == evidence['sign_model_sha256']
    dest = OUT / 'restart2-fence/sign-blockers-v2'
    dest.mkdir(exist_ok=False)
    results = []
    for row in assembly['instances']:
        index = row['target_index']
        if index == 4:
            continue
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.context.scene
        scene.frame_set(1)
        selected = set(row['parts'])
        for obj in list(scene.objects):
            if obj.type == 'MESH' and (obj.name not in selected or 'native_frame' in obj):
                bpy.data.objects.remove(obj, do_unlink=True)
        mat = bpy.data.materials.new('Opaque sign-body diagnostic')
        mat.use_nodes = True
        mat.node_tree.nodes.clear()
        emit = mat.node_tree.nodes.new('ShaderNodeEmission')
        emit.inputs[0].default_value = (1, 0, 1, 1)
        out = mat.node_tree.nodes.new('ShaderNodeOutputMaterial')
        mat.node_tree.links.new(emit.outputs[0], out.inputs[0])
        for obj in scene.objects:
            if obj.type == 'MESH' and obj.scale.x > .5:
                for slot in obj.material_slots:
                    slot.material = mat
        groups = {}
        for key in next(r for r in evidence['results'] if r['target_index'] == index)['neighbors']:
            source = evidence['inputs'][key]
            path = Path(source['worker']) / 'model.blend'
            assert sha(path) == source['model_sha256']
            with bpy.data.libraries.load(str(path), link=False) as (src, data):
                data.objects = list(source['objects'])
            groups[key] = list(data.objects)
            for obj in groups[key]:
                scene.collection.objects.link(obj)
                before = signature(obj)
                world = obj.matrix_world.copy()
                obj.parent = None
                obj.matrix_world = world
                obj.hide_render = True
                assert before == signature(obj)
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.use_denoising = False
        scene.cycles.pixel_filter_type = 'BOX'
        scene.cycles.filter_width = .01
        scene.cycles.seed = 0
        scene.cycles.use_adaptive_sampling = False
        scene.render.use_compositing = False
        scene.render.dither_intensity = 0
        scene.cycles.transparent_max_bounces = 128
        scene.render.film_transparent = True
        scene.render.image_settings.color_mode = 'RGBA'
        scene.render.resolution_x = 288
        scene.render.resolution_y = 288
        scene.render.resolution_percentage = 100
        scene.view_settings.view_transform = 'Standard'
        scene.view_settings.look = 'None'
        data = bpy.data.cameras.new('Source sign blocker camera')
        data.type = 'ORTHO'
        data.sensor_fit = 'HORIZONTAL'
        data.ortho_scale = 96
        data.clip_end = 20000
        camera = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(camera)
        scene.camera = camera
        x, y = row['native_target']['position_x'], row['native_target']['position_y']
        camera_to(camera, Vector((x, -(y-16)/SIN, 0)), RAY)
        alone = body(render(scene, dest / f'target-{index}-alone.png'))
        expected = body(Image.open(base / f'target-{index}/pose-00-body-alone.png').convert('RGBA'))
        assert np.array_equal(alone, expected)
        blockers = []
        for key, objects in groups.items():
            for obj in objects:
                obj.hide_render = False
            actual = body(render(scene, dest / f'target-{index}-{key}.png'))
            blocked = alone & ~actual
            blockers.append(dict(asset_key=key, blocked_body_pixels=int(blocked.sum()),
                                 model_sha256=evidence['inputs'][key]['model_sha256'],
                                 objects=[o.name for o in objects]))
            for obj in objects:
                obj.hide_render = True
        results.append(dict(target_index=index, pose=0, physical_body_pixels=int(alone.sum()), blockers=blockers))
    assert sha(model) == evidence['sign_model_sha256']
    assert all(sha(Path(r['worker']) / 'model.blend') == r['model_sha256'] for r in evidence['inputs'].values())
    write_json(dest / 'report.json', dict(source_manifest_sha256=sha(base / 'manifest.json'),
               sign_model_sha256=sha(model), results=results,
               limitations=['Per-asset blockers may overlap; counts do not sum to the full-neighborhood union.',
                            'Pose0 diagnostic only; no source model changes or neighbor geometry approval implied.']))
    print(dest)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
