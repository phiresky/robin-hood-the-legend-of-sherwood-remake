"""Reopen one York candidate and inspect complete bounds and source placement."""
import hashlib
import argparse
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workspace', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--crop', nargs=4, type=int, metavar=('LEFT', 'TOP', 'RIGHT', 'BOTTOM'))
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = args.workspace.resolve()
    destination = args.output.resolve() if args.output else workspace / 'inspection-v1'
    if destination.exists():
        raise FileExistsError(destination)
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
    from freeze_tooling import select_tooling
    select_tooling(json.loads((OUT / 'tooling/current.json').read_text())['directory'])
    import bpy
    from mathutils import Vector
    from refinement_review import render_review
    from render_multiview_asset import render
    from render_views import render_views
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    config = json.loads((workspace / 'workspace.json').read_text())
    scene = bpy.data.scenes[config['scene_name']]
    bpy.context.window.scene = scene
    destination.mkdir()
    render_review(destination / 'complete-object', scene_name=scene.name,
                  collection_name=config['collection_name'], asset_id=config['asset_id'],
                  source_path=config['source_path'], width=384, height=512,
                  framing_padding=1.15, lighting=config['lighting'])
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 16
    scene.cycles.use_denoising = False
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 2
    scene.render.film_transparent = True
    scene.view_settings.view_transform = 'Standard'
    scene.view_settings.look = 'None'
    scene.view_settings.exposure = 0
    scene.view_settings.gamma = 1
    render(destination / 'complete-object/views.json', destination / 'actual', width=384)
    # Source crop uses one world unit per native pixel and the exact map elevation.
    complete = json.loads((destination / 'complete-object/views.json').read_text())
    bounds = complete['context_crop']
    crop = args.crop or [bounds[k] for k in ('left', 'top', 'right', 'bottom')]
    w, h = crop[2] - crop[0], crop[3] - crop[1]
    if w <= 0 or h <= 0:
        raise ValueError('Native inspection crop must have positive dimensions')
    elevation = math.radians(35)
    center = Vector(((crop[0] + crop[2]) / 2,
                     -(crop[1] + crop[3]) / 2 / math.sin(elevation), 0))
    backward = Vector((0, -math.cos(elevation), math.sin(elevation)))
    data = bpy.data.cameras.new('York native candidate inspection')
    camera = bpy.data.objects.new(data.name, data)
    scene.collection.objects.link(camera)
    data.type = 'ORTHO'
    data.ortho_scale = max(w, h)
    data.clip_start, data.clip_end = .01, 20000
    camera.location = center + backward * 10000
    camera.rotation_euler = (-backward).to_track_quat('-Z', 'Y').to_euler()
    scene.render.resolution_x, scene.render.resolution_y = w * 4, h * 4
    meshes = [o for o in scene.objects if o.type == 'MESH']
    visibility = [(o, o.hide_render) for o in meshes]
    for obj, hidden in visibility:
        obj.hide_render = hidden or obj.get('asset_group') != config['asset_id']
    render_views(scene.name, {'native': camera.name}, destination / 'native-isolated',
                 width=w * 4, modes=('textured',))
    for obj, hidden in visibility:
        obj.hide_render = hidden
    render_views(scene.name, {'native': camera.name}, destination / 'native-joint',
                 width=w * 4, modes=('textured',))
    record = {'model_sha256': hashlib.sha256((workspace / 'model.blend').read_bytes()).hexdigest(),
              'asset_id': config['asset_id'], 'source_crop': crop, 'source_scale': 4,
              'scope': 'Actual reopened saved materials, eight complete-object views and exact native isolated/joint views.',
              'status': 'Awaiting independent visual and source-coverage review.'}
    (destination / 'evidence.json').write_text(json.dumps(record, indent=2) + '\n')


if __name__ == '__main__':
    main()
