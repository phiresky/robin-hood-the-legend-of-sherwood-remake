"""Reopen one York candidate and inspect complete bounds and source placement."""
import hashlib
import argparse
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'


def geometric_first_hits(scene, asset_id, crop, destination):
    """Diagnose opaque triangle competition; alpha requires separate review."""
    import numpy as np
    left, top, right, bottom = crop
    shape = (bottom - top, right - left)
    depth = np.full(shape, -np.inf)
    owner = np.full(shape, -1, dtype=np.int32)
    target_depth = np.full(shape, -np.inf)
    names = []
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    for obj in scene.objects:
        if obj.type != 'MESH' or obj.hide_render:
            continue
        name = obj.get('source_node', obj.name)
        index = len(names)
        names.append(name)
        points = np.array([tuple(obj.matrix_world @ v.co) for v in obj.data.vertices])
        if not len(points):
            continue
        pixels = np.column_stack((points[:, 0], -points[:, 1] * sine - points[:, 2] * cosine))
        if pixels[:, 0].max() < left or pixels[:, 0].min() > right or pixels[:, 1].max() < top or pixels[:, 1].min() > bottom:
            continue
        distances = -points[:, 1] * cosine + points[:, 2] * sine
        obj.data.calc_loop_triangles()
        for triangle in obj.data.loop_triangles:
            ids = list(triangle.vertices)
            a, b, c = pixels[ids]
            x0 = max(left, int(math.floor(min(a[0], b[0], c[0]))))
            x1 = min(right, int(math.ceil(max(a[0], b[0], c[0]))))
            y0 = max(top, int(math.floor(min(a[1], b[1], c[1]))))
            y1 = min(bottom, int(math.ceil(max(a[1], b[1], c[1]))))
            denom = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
            if x0 >= x1 or y0 >= y1 or abs(denom) < 1e-8:
                continue
            yy, xx = np.mgrid[y0:y1, x0:x1]
            xx, yy = xx + .5, yy + .5
            u = ((b[1]-c[1])*(xx-c[0])+(c[0]-b[0])*(yy-c[1]))/denom
            v = ((c[1]-a[1])*(xx-c[0])+(a[0]-c[0])*(yy-c[1]))/denom
            inside = (u >= -1e-7) & (v >= -1e-7) & (u+v <= 1+1e-7)
            z = u*distances[ids[0]]+v*distances[ids[1]]+(1-u-v)*distances[ids[2]]
            region = np.s_[y0-top:y1-top, x0-left:x1-left]
            nearer = inside & (z > depth[region])
            depth[region][nearer] = z[nearer]
            owner[region][nearer] = index
            if obj.get('asset_group') == asset_id:
                nearer = inside & (z > target_depth[region])
                target_depth[region][nearer] = z[nearer]
    covered = np.isfinite(target_depth)
    blocked = covered & (depth > target_depth + .05)
    counts = {}
    for index in np.unique(owner[blocked]):
        counts[names[index]] = counts.get(names[index], 0) + int(np.sum(blocked & (owner == index)))
    (destination / 'geometric-first-hits.json').write_text(json.dumps({
        'scope': 'Saved visible mesh triangles at native pixel centres. Opaque diagnostic only; no material alpha or source-ownership inference.',
        'source_crop': crop, 'isolated_candidate_pixels': int(covered.sum()),
        'blocked_by_other_geometry': int(blocked.sum()),
        'blocking_source_nodes': dict(sorted(counts.items(), key=lambda row: -row[1])),
    }, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('workspace', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--crop', nargs=4, type=int, metavar=('LEFT', 'TOP', 'RIGHT', 'BOTTOM'))
    parser.add_argument('--audit-only', action='store_true', help='Write native geometric blocking diagnostics without renders; requires --crop')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = args.workspace.resolve()
    destination = args.output.resolve() if args.output else workspace / 'inspection-v1'
    if args.audit_only and not args.crop:
        parser.error('--audit-only requires --crop')
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
    if args.audit_only:
        geometric_first_hits(scene, config['asset_id'], args.crop, destination)
        return
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
    geometric_first_hits(scene, config['asset_id'], crop, destination)
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
