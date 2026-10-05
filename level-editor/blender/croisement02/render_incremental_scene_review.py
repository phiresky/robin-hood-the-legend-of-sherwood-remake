"""Review a pinned private assembly with native and oblique transparency checks."""
import argparse
import html
import json
import math
import shutil
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from PIL import Image

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[1] / 'refinement'))
sys.path.insert(0, str(HERE.parents[1] / 'refinement/blender'))
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from stage_review_scene import render_review
from tree_geometry import SIN, RAY


def render(scene, path, budget):
    scene.cycles.transparent_max_bounces = budget
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True, scene=scene.name)


def difference(first, second):
    a = np.asarray(Image.open(first).convert('RGBA')).astype(np.int16)
    b = np.asarray(Image.open(second).convert('RGBA')).astype(np.int16)
    delta = np.abs(a - b)
    return dict(first_sha256=sha(first), second_sha256=sha(second),
                changed_pixels=int(np.any(delta, axis=2).sum()), maximum_channel_delta=int(delta.max()),
                mean_channel_delta=float(delta.mean()))


def main(stage, output):
    assembly = json.loads((stage / 'assembly.json').read_text())
    model = stage / 'scene.blend'
    if sha(model) != assembly['model_sha256'] or output.exists():
        raise ValueError('Stale assembly or existing output')
    acquire()
    try:
        output.mkdir(parents=True)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.data.scenes['Croisement02 Refinement']
        bpy.context.window.scene = scene
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 4
        scene.cycles.use_denoising = False
        scene.cycles.seed = 0
        scene.render.resolution_x, scene.render.resolution_y = 1792, 1152
        scene.render.resolution_percentage = 100
        scene.render.image_settings.file_format = 'PNG'
        scene.render.film_transparent = False
        scene.view_settings.view_transform = 'Standard'
        scene.view_settings.look = 'None'
        data = bpy.data.cameras.new('Native whole-map review')
        data.type = 'ORTHO'
        data.ortho_scale = 1792
        data.clip_end = 20000
        camera = bpy.data.objects.new(data.name, data)
        scene.collection.objects.link(camera)
        scene.camera = camera
        center = Vector((896, -576 / SIN, 0))
        camera.location = center + RAY * 6000
        camera.rotation_euler = (center - camera.location).to_track_quat('-Z', 'Y').to_euler()
        native = []
        previous = None
        budget = None
        for current in (256, 512, 1024):
            path = output / f'native-{current}.png'
            render(scene, path, current)
            if previous:
                comparison = difference(previous, path)
                native.append(dict(budget=current, **comparison))
                if comparison['changed_pixels'] == 0:
                    budget = current // 2
                    break
            previous = path
        native_converged = budget is not None
        if budget is None:
            # Still produce the requested diagnostic views, explicitly held.
            budget = 1024
        shutil.copyfile(output / f'native-{budget}.png', output / 'native.png')
        source = OUT / 'animation-references/composite-frame-0.png'
        shutil.copyfile(source, output / 'native-source.png')
        orbit = output / f'orbit-{budget}'
        orbit.mkdir()
        render_review(scene, orbit, assembly['model_sha256'], transparent_bounces=budget)
        evidence = json.loads((orbit / 'render-evidence.json').read_text())
        view = evidence['views'][1]
        scene.camera.location = view['location']
        scene.camera.rotation_euler = view['rotation']
        scene.camera.data.ortho_scale = view['ortho_scale']
        comparison_budget = budget * 2 if budget < 1024 else 512
        high = output / f'oblique1-{comparison_budget}.png'
        render(scene, high, comparison_budget)
        oblique = difference(orbit / 'view-1.png', high)
        converged = native_converged and oblique['changed_pixels'] == 0
        write_json(output / 'convergence.json', dict(status='PASS' if converged else 'HOLD',
                   native=native, native_converged=native_converged,
                   representative_oblique=oblique, oblique_budgets=[budget, comparison_budget], selected_budget=budget,
                   note='Existing approved renders were not changed. Nonconvergence blocks readiness.'))
        report = dict(status='ready for independent full-scene review' if converged else 'HOLD transparency convergence',
                      model_sha256=assembly['model_sha256'], assembly_sha256=sha(stage / 'assembly.json'),
                      orbit=str(orbit), orbit_evidence_sha256=sha(orbit / 'render-evidence.json'),
                      native_render_sha256=sha(output / 'native.png'), source=str(source), source_sha256=sha(source),
                      counts=assembly['counts'], later_selector_delta=assembly['later_selector_delta'],
                      pending_replacements=assembly['pending_replacements'], state_scope=assembly['state_scope'],
                      user_approval=None, publication='not performed')
        write_json(output / 'review.json', report)
        pending = html.escape(json.dumps(dict(later_selector_delta=assembly['later_selector_delta'],
                              pending_replacements=assembly['pending_replacements'], state_scope=assembly['state_scope']), indent=2))
        (output / 'index.html').write_text('<!doctype html><meta charset="utf-8"><title>Croisement02 private126 review</title>'
            '<style>body{background:#252525;color:#eee;font:16px system-ui;margin:24px}img{max-width:100%;height:auto}pre{white-space:pre-wrap}</style>'
            '<h1>Croisement02 private126 spatial review</h1><p>122 visible groups,4 state-only groups;24 exact approved texture models. '
            'Pending geometry and unknown surfaces are shown for cross-asset review. This is not a completed state integration or publication.</p>'
            f'<p>{html.escape(report["status"])}</p><h2>All eight saved-model views</h2><img src="{orbit.name}/sheet.png">'
            '<h2>Native camera</h2><img src="native.png"><h2>Native source artwork</h2><img src="native-source.png">'
            f'<h2>Explicit omissions and later revisions</h2><pre>{pending}</pre>')
        if sha(model) != assembly['model_sha256']:
            raise ValueError('Rendering changed saved assembly')
        print(output / 'index.html', flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.stage.resolve(), args.output.resolve())
