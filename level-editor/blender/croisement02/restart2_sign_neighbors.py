"""Read-only physical neighborhood proof for the five native sign placements."""
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from catalog import OUT, scenery_workspace, tree_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY
from stage_review_scene import signature
from sign_object_mask import setup as setup_object_mask

NEIGHBORS = {
    4: ['east-stone-wall-and-gate', 'east-rail-fence', 'shrub-74'],
    5: ['southwest-field-wattle-fence', 'shrub-77'],
    6: ['north-woodland-bank', 'west-shrub-bank', *range(24, 30)],
    7: ['north-woodland-bank', 'west-rock-outcrop', 'southwest-rock-outcrop',
        'northwest-boundary-shrub-54', 'shrub-57', 2],
    8: ['north-woodland-bank', 14, 15, 16, 17, 18],
}


def render(scene, path):
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    return Image.open(path).convert('RGBA')


def camera_to(camera, center, direction):
    camera.location = center + direction * 5000
    camera.rotation_euler = (center - camera.location).to_track_quat('-Z', 'Y').to_euler()


def main():
    dest = OUT / 'restart2-fence/sign-neighbors-v4'
    dest.mkdir(parents=True, exist_ok=False)
    base = OUT / 'state-sign-candidate'
    assembly = json.loads((base / 'five-instances-v3/assembly.json').read_text())
    model = base / 'five-instances-v3/model.blend'
    assert sha(model) == assembly['model_sha256']
    order_path = base / 'native-order-reference-v3/manifest.json'
    order = json.loads(order_path.read_text())
    background = Image.open(OUT / 'baseline/covered.png').convert('RGBA')
    frames = next(p for p in json.loads((OUT / 'state-target-evidence/manifest.json').read_text())['profiles']
                  if p['id'] == 'TG_Panel-12')['rows'][0]['frames']
    animations = json.loads((OUT / 'animation-references/manifest.json').read_text())['animations']
    inputs = {}
    for key in dict.fromkeys(v for values in NEIGHBORS.values() for v in values):
        worker = tree_workspace(key) if isinstance(key, int) else scenery_workspace('croisement02-' + key)
        audit_path = worker / 'inspection/saved-model-audit.json'
        audit = json.loads(audit_path.read_text())
        digest = sha(worker / 'model.blend')
        assert audit['status'] == 'PASS' and audit['model_sha256'] == digest
        inputs[key] = dict(worker=str(worker), model_sha256=digest,
                           audit_sha256=sha(audit_path), objects=[r['object'] for r in audit['objects']])
    results = []
    for row in assembly['instances']:
        index = row['target_index']
        target_dir = dest / f'target-{index}'
        target_dir.mkdir()
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.context.scene
        selected = set(row['parts'])
        for obj in list(scene.objects):
            if obj.type == 'MESH' and obj.name not in selected:
                bpy.data.objects.remove(obj, do_unlink=True)
        neighbors = []
        for key in NEIGHBORS[index]:
            source = inputs[key]
            with bpy.data.libraries.load(str(Path(source['worker']) / 'model.blend'), link=False) as (src, data):
                data.objects = list(source['objects'])
            for obj in data.objects:
                scene.collection.objects.link(obj)
                before = signature(obj)
                world = obj.matrix_world.copy()
                obj.parent = None
                obj.matrix_world = world
                obj.hide_render = False
                assert signature(obj) == before
                neighbors.append(obj)
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
        setup_object_mask(scene, [scene.objects[n] for n in selected if 'native_body_frame' in scene.objects[n]], neighbors)
        data = bpy.data.cameras.new('Physical sign neighborhood camera')
        data.type = 'ORTHO'
        data.sensor_fit = 'HORIZONTAL'
        data.ortho_scale = 96
        data.clip_end = 20000
        camera = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(camera)
        scene.camera = camera
        t = row['native_target']
        x, y = t['position_x'], t['position_y']
        box = (x-48, y-64, x+48, y+32)
        center = Vector((x, -(y-16)/SIN, 0))
        camera_to(camera, center, RAY)
        source_order = next(r for r in order['records'] if r['target_index'] == index)
        diagnostics = []
        sheet = Image.new('RGB', (864, 4*312), (70, 70, 70))
        for ordinal, phase in enumerate([0, 8, 16, 24]):
            scene.frame_set(1+phase*2)
            scene.render.use_compositing = False
            actual = render(scene, target_dir / f'pose-{phase:02}-actual.png')
            native = background.crop(box).resize((288, 288), Image.Resampling.NEAREST)
            f = frames[phase]
            sprite = Image.open(f['image']).convert('RGBA')
            native.alpha_composite(sprite.resize((sprite.width*3, sprite.height*3), Image.Resampling.NEAREST),
                                   ((48+int(f['offset'][0]))*3, (64+int(f['offset'][1]))*3))
            for overlap in source_order['overlapping_animations']:
                assert overlap['after_sign']
                frame = next(a for a in animations if a['index'] == overlap['index'])['frames'][0]
                fx, fy, fw, fh = frame['bbox']
                sprite = Image.open(frame['image']).convert('RGBA').resize((fw*3, fh*3), Image.Resampling.NEAREST)
                native.alpha_composite(sprite, ((fx-box[0])*3, (fy-box[1])*3))
            display = background.crop(box).resize((288, 288), Image.Resampling.NEAREST)
            display.alpha_composite(actual)
            sheet.paste(native.convert('RGB'), (0, ordinal*312))
            sheet.paste(actual, (288, ordinal*312), actual.getchannel('A'))
            sheet.paste(display.convert('RGB'), (576, ordinal*312))
            ImageDraw.Draw(sheet).text((3, ordinal*312+290),
                                      f'Pose {phase}: native / physical alpha / physical over static background', fill='white')
            bodies = [scene.objects[n] for n in selected
                      if 'native_body_frame' in scene.objects[n] and scene.objects[n].scale.x > .5]
            shadows = [scene.objects[n] for n in selected if 'native_frame' in scene.objects[n]]
            assert len(bodies) == 2
            scene.render.use_compositing = True
            for obj in shadows:
                obj.hide_render = True
            joint = np.asarray(render(scene, target_dir / f'pose-{phase:02}-body-first-hit.png'))[1::3, 1::3]
            for obj in neighbors:
                obj.hide_render = True
            alone = np.asarray(render(scene, target_dir / f'pose-{phase:02}-body-alone.png'))[1::3, 1::3]
            def body(a):
                return a[:, :, 0] > 127
            before, after = body(alone), body(joint)
            diagnostics.append(dict(sign_pose=phase, physical_body_pixels=int(before.sum()),
                                    physical_body_hidden_by_neighbors=int((before & ~after).sum())))
            for obj in neighbors:
                obj.hide_render = False
            for obj in shadows:
                obj.hide_render = False
            scene.render.use_compositing = False
        sheet.save(target_dir / 'native-physical-comparison.png')
        scene.frame_set(1)
        center = Vector(row['world_anchor']) + Vector((0, 0, 22))
        for i, angle in enumerate([-math.pi/4, math.pi/4]):
            camera_to(camera, center, Vector((math.sin(angle)*COS, -math.cos(angle)*COS, SIN)))
            render(scene, target_dir / f'oblique-{i}.png')
        result = dict(target_index=index, neighbors=[str(k) for k in NEIGHBORS[index]],
                      diagnostics=diagnostics, source_order=source_order['insertion'],
                      native_overlays=source_order['overlapping_animations'])
        results.append(result)
        write_json(target_dir / 'report.json', result)
    assert sha(model) == assembly['model_sha256']
    assert all(sha(Path(r['worker']) / 'model.blend') == r['model_sha256'] for r in inputs.values())
    previous = json.loads((OUT / 'restart2-fence/sign-neighbors-v3/manifest.json').read_text())
    changes = {str(k): dict(previous=previous['inputs'].get(str(k), {}).get('model_sha256'), current=v['model_sha256'])
               for k, v in inputs.items() if previous['inputs'].get(str(k), {}).get('model_sha256') != v['model_sha256']}
    write_json(dest / 'manifest.json', dict(
        status='Read-only scoped physical neighborhood proof; visual assessment pending',
        sign_model_sha256=assembly['model_sha256'], native_order_sha256=sha(order_path),
        inputs={str(k): v for k, v in inputs.items()}, results=results,
        diagnostic_sampling='Object-index777 compositor mask; alpha threshold0.5, antialiasing off. Actual appearances are separate untouched-material renders. BOX0.01, fixed seed0, adaptive sampling/dither off.',
        selector_model_changes_since_v3=changes,
        limitations=['Neighbor census uses catalog asset source bounds; this is not the complete staged map.',
                     'Native reference uses overlay frame0. Physical tree hypotheses are static and require separate phase handling.',
                     'Native butterfly is shown in reference only; no physical ambient actor is fabricated.',
                     'Background-composited column is illustrative. Transparent physical column and first-hit diagnostic are the actual geometry proof.',
                     'Four cardinal sign poses sampled; the complete32-pose animation proof is separate.',
                     'No source model, canonical selector, approval, or publication is changed.']))
    print(dest)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
