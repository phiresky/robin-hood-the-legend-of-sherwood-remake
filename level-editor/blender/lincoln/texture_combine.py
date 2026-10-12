"""Fill texels left neutral by the global source reprojection from reviewed generated sheets.

Run in Blender from the repository root (never promotes anything):

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/texture_combine.py -- \
      --worker-in <publication-N/stage-vK/worker.blend> --output <new directory> \
      [--asset ID ...] [--no-render]

The reprojected worker is the base. Its exterior ownership atlases and UV layers are kept.
Only texels that still hold the bake's neutral shade (`global_reproject.neutral_mask`) are
written. For each asset with a ready texture candidate (`texture-review.json` with status
ready-for-user under textures/experiments or textures/variants), each such texel takes its
color from the candidate's `generated-preserved.png` (source pixels restored, inferred pixels
generated). The texel's world position and triangle normal come from the same island
rasterization as the reprojection. Of the asset's eight fixed review cameras, the view with
the highest facing cosine wins (ties: lower view index), provided the texel is first-hit
visible against the asset's own meshes in that view and projects inside the reviewed
silhouette. Views seeing the texel at facing cosine below the reprojection's grazing floor (0.05) are never sampled. Texels the
reprojection reset below its grazing floor (fill-mask value 3) were source-known in the
reviewed packet, so the preserved sheet still holds stretched source there: they only accept
pixels the approved mask marks editable. Neutral texels no view can fill stay neutral and are
counted (unseen, grazing_unfilled).

Before any texel is written, each asset's triangles are projected into every view and the
silhouette must match the approved solid sheet (>= 97% agreement, 1-px boundary tolerance). This proves the published
transforms equal the reviewed cameras' world. Geometry, UVs, material graphs, slot
assignments and non-neutral texels are verified unchanged. The output worker, combine.json
(per asset/object counts and hashes) and, unless --no-render, each asset's eight actual
review views (renders/<id>/textured.png) are written to --output.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[3]
TEXTURES = ROOT / 'level-editor/work/needs-review/lincoln-refinement/textures'
GENERATION = 'generation-short-no-mask-with-lighting-openrouter'
VIEW_MIN_COSINE = 0.05  # Match the global reprojection's grazing floor (--grazing-cosine 0.05).
GRAZING_RESET = 3  # global_reproject fill-mask value: bake source texel reset below its grazing floor.
DEPTH_TOLERANCE = 0.02
SUPERSAMPLE = 2
SILHOUETTE_IOU = 0.97


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def approved_revisions():
    """asset id -> set of texture-approved review revisions (decisions.json, scope texture)."""
    path = TEXTURES / 'decisions.json'
    records = json.loads(path.read_text())['decisions'] if path.exists() else []
    result = {}
    for record in records:
        if record.get('scope') == 'texture' and record.get('decision') == 'approved':
            result.setdefault(record['asset_id'], []).append(record)
    return result


def candidates(approved_only=True):
    """asset id -> ready experiment directory; at most one per asset.

    With approved_only, the experiment's baked model and actual sheet must equal the evidence
    bound by a recorded texture approval.
    """
    approvals = approved_revisions() if approved_only else None
    selected = {}
    for root in (TEXTURES / 'experiments', TEXTURES / 'variants', TEXTURES / 'retries'):
        for review_path in sorted(root.glob('*/texture-review.json')):
            review = json.loads(review_path.read_text())
            if review.get('status') != 'ready-for-user':
                continue
            experiment = review_path.parent
            asset_id = json.loads((experiment / 'approval.json').read_text())['asset_id']
            if approvals is not None:
                bake = experiment / review['bake']
                current = (sha(bake / 'worker.blend'), sha(bake / 'actual/textured.png'))
                if not any((r['evidence_sha256'].get('model'), r['evidence_sha256'].get('textured')) == current
                           for r in approvals.get(asset_id, [])):
                    continue
            if asset_id in selected:
                raise ValueError('Multiple ready texture candidates for ' + asset_id)
            selected[asset_id] = experiment
    return selected


def load_rgba(path):
    from PIL import Image
    return np.asarray(Image.open(path).convert('RGBA'))  # top-origin rows


class View:
    """One fixed orthographic review camera in sheet pixel coordinates (top-origin rows)."""

    def __init__(self, view, sheet_height):
        from texture_camera import orthographic_extents
        self.index = view['index']
        matrix = np.array(view['camera_matrix_world'], dtype=np.float64)
        self.inverse = np.linalg.inv(matrix)
        self.toward = matrix[:3, :3] @ np.array([0.0, 0.0, 1.0])  # from surface to camera
        self.crop = view['crop']
        self.extent = orthographic_extents(view)

    def project(self, points):
        """World -> (sheet x, top-origin sheet y, depth toward camera)."""
        local = points @ self.inverse[:3, :3].T + self.inverse[:3, 3]
        horizontal, vertical = self.extent
        c = self.crop
        x = c['left'] + (.5 + local[:, 0] / horizontal) * c['width']
        y = c['top'] + (.5 - local[:, 1] / vertical) * c['height']
        return x, y, local[:, 2]


def zbuffer(view, corners):
    """Nearest-surface depth (largest camera-space z) per supersampled tile pixel, and coverage."""
    c = view.crop
    W, H = c['width'] * SUPERSAMPLE, c['height'] * SUPERSAMPLE
    x, y, d = view.project(corners.reshape(-1, 3))
    xs = ((x - c['left']) * SUPERSAMPLE).reshape(-1, 3)
    ys = ((y - c['top']) * SUPERSAMPLE).reshape(-1, 3)
    d = d.reshape(-1, 3)
    depth = np.full((H, W), -np.inf)
    area = (xs[:, 1] - xs[:, 0]) * (ys[:, 2] - ys[:, 0]) - (xs[:, 2] - xs[:, 0]) * (ys[:, 1] - ys[:, 0])
    x0 = np.maximum(0, np.ceil(xs.min(1) - .5)).astype(int)
    x1 = np.minimum(W - 1, np.floor(xs.max(1) - .5)).astype(int)
    y0 = np.maximum(0, np.ceil(ys.min(1) - .5)).astype(int)
    y1 = np.minimum(H - 1, np.floor(ys.max(1) - .5)).astype(int)
    for t in np.flatnonzero((np.abs(area) > 1e-12) & (x1 >= x0) & (y1 >= y0)):
        gx = np.arange(x0[t], x1[t] + 1) + .5
        gy = (np.arange(y0[t], y1[t] + 1) + .5)[:, None]
        (ax, bx, cx), (ay, by, cy) = xs[t], ys[t]
        w0 = ((bx - gx) * (cy - gy) - (cx - gx) * (by - gy)) / area[t]
        w1 = ((cx - gx) * (ay - gy) - (ax - gx) * (cy - gy)) / area[t]
        w2 = 1 - w0 - w1
        inside = (w0 >= -1e-9) & (w1 >= -1e-9) & (w2 >= -1e-9)
        z = w0 * d[t, 0] + w1 * d[t, 1] + w2 * d[t, 2]
        window = depth[y0[t]:y1[t] + 1, x0[t]:x1[t] + 1]
        take = inside & (z > window)
        window[take] = z[take]
    return depth


def dilate(mask):
    out = mask.copy()
    out[1:] |= mask[:-1]; out[:-1] |= mask[1:]
    grown = out.copy()
    grown[:, 1:] |= out[:, :-1]; grown[:, :-1] |= out[:, 1:]
    return grown


def silhouette_iou(depth, solid_tile):
    """Silhouette agreement tolerating 1-pixel boundary differences (thin fences, rails)."""
    covered = np.isfinite(depth)
    # Downsample supersampled coverage: a tile pixel is covered if any subsample is.
    h, w = solid_tile.shape
    covered = covered.reshape(h, SUPERSAMPLE, w, SUPERSAMPLE).any(axis=(1, 3))
    union = (covered | solid_tile).sum()
    mismatch = ((covered & ~dilate(solid_tile)) | (solid_tile & ~dilate(covered))).sum()
    return float(1 - mismatch / union) if union else 1.0


def visible(view, depth, points):
    c = view.crop
    x, y, d = view.project(points)
    fx, fy = (x - c['left']) * SUPERSAMPLE - .5, (y - c['top']) * SUPERSAMPLE - .5
    H, W = depth.shape
    result = np.zeros(len(points), dtype=bool)
    for ox in (0, 1):
        for oy in (0, 1):
            cx = np.floor(fx).astype(int) + ox
            cy = np.floor(fy).astype(int) + oy
            inside = (cx >= 0) & (cx < W) & (cy >= 0) & (cy < H)
            front = np.where(inside, depth[np.clip(cy, 0, H - 1), np.clip(cx, 0, W - 1)], -np.inf)
            result |= inside & (front <= d + DEPTH_TOLERANCE)
    return result, x, y


def sample(sheet, solid, view, x, y, editable=None, require_editable=None):
    """Bilinear sample inside the silhouette; nearest when the footprint touches background."""
    c = view.crop
    fx, fy = x - .5, y - .5
    ix, iy = np.floor(fx).astype(int), np.floor(fy).astype(int)
    tx, ty = fx - ix, fy - iy
    left, top = c['left'], c['top']
    right, bottom = left + c['width'] - 1, top + c['height'] - 1
    color = np.zeros((len(x), 3))
    support = np.ones(len(x), dtype=bool)
    for ox, oy, weight in ((0, 0, (1 - tx) * (1 - ty)), (1, 0, tx * (1 - ty)),
                           (0, 1, (1 - tx) * ty), (1, 1, tx * ty)):
        px, py = np.clip(ix + ox, left, right), np.clip(iy + oy, top, bottom)
        color += sheet[py, px, :3] * weight[:, None]
        support &= solid[py, px]
    nx, ny = np.clip(np.floor(x).astype(int), left, right), np.clip(np.floor(y).astype(int), top, bottom)
    color[~support] = sheet[ny[~support], nx[~support], :3]
    ok = solid[ny, nx]
    if require_editable is not None:
        # Grazing-reset texels: the reviewed packet treated these faces as source-known, so the
        # preserved sheet still holds the stretched source there. Accept generated pixels only.
        ok &= ~require_editable | editable[ny, nx]
    return np.rint(color).astype(np.uint8), ok


def combine_asset(asset_id, experiment, scene, meshes, gr, fill_masks=None):
    """Write generated samples into still-neutral exterior texels of one asset's meshes."""
    manifest = json.loads((experiment / 'views.json').read_text())
    review = json.loads((experiment / 'texture-review.json').read_text())
    generation = experiment / review['generation']
    preserved = generation / 'generated-preserved.png'
    report = json.loads((generation / 'generation.json').read_text())
    if report.get('changedProtected') != 0:
        raise ValueError('Generated sheet changed protected pixels: ' + asset_id)
    sheet = load_rgba(preserved).astype(np.float64)
    solid = load_rgba(experiment / 'solid.png')[..., 3] > 0
    editable = load_rgba(experiment / 'mask.png')[..., 3] == 0
    if sheet.shape[:2] != solid.shape:
        raise ValueError('Generated sheet and solid sheet differ in size: ' + asset_id)
    records = [r for r in meshes if r['object'].get('asset_group') == asset_id]
    # Publication renames objects to catalog part names; the mesh datablock keeps the
    # approved object name (plus a Blender ".NNN" suffix). Map approved objects through it.
    def approved_name(record):
        obj = record['object']
        if obj.name in expected:
            return obj.name
        base = re.sub(r'\.\d{3}$', '', obj.data.name)
        return base if base in expected else None
    expected = set(manifest.get('render_object_names') or manifest['object_names'])
    mapped = {}
    for record in records:
        name = approved_name(record)
        if name is not None:
            if name in mapped:
                raise ValueError(f'{asset_id}: two worker meshes map to approved object {name}')
            mapped[name] = record
    # Fallback for renamed mesh datablocks: an approved name ending in a node number maps to
    # the single unmapped worker mesh with that source_node (the silhouette check still applies).
    for name in sorted(expected - set(mapped)):
        match = re.search(r'(\d{3})$', name)
        rest = [r for r in records if r not in mapped.values()]
        same = [r for r in rest if match and r['object'].get('source_node') == 'building-' + match.group(1)]
        pending = [n for n in expected - set(mapped) if n.endswith(match.group(1))] if match else []
        if len(same) == 1 and len(pending) == 1:
            mapped[name] = same[0]
    if set(mapped) != expected:
        raise ValueError(f'{asset_id}: approved objects absent from worker: {sorted(expected - set(mapped))}')
    records = list(mapped.values())
    corners = np.concatenate([r['corners'] for r in records])
    views, depths, ious = [], [], {}
    for entry in manifest['views']:
        view = View(entry, sheet.shape[0])
        depth = zbuffer(view, corners)
        c = view.crop
        ious[view.index] = silhouette_iou(depth, solid[c['top']:c['top'] + c['height'], c['left']:c['left'] + c['width']])
        views.append(view)
        depths.append(depth)
    if min(ious.values()) < SILHOUETTE_IOU:
        raise ValueError(f'{asset_id}: published geometry does not match reviewed cameras: {ious}')
    objects = {}
    for record in records:
        obj = record['object']
        mesh = obj.data
        counts = {'neutral_before': 0, 'generated': 0, 'unseen': 0}
        for slot in sorted(set(record['slots'].tolist())):
            binding = scene.slot_binding(obj, slot)
            if binding is None or binding['kind'] != 'ownership':
                continue
            image = binding['image']
            atlas = gr.read_image(image)  # bottom-origin rows
            before = atlas.copy()
            uv = gr.slot_uvs(obj, binding['uv'])
            fill = None
            if fill_masks is not None:
                key = hashlib.sha256((obj.name + '\0' + str(slot)).encode()).hexdigest()[:20]
                path = fill_masks / (key + '.npz')
                if path.exists():
                    fill = np.load(path)['fill']
                    if fill.shape != atlas.shape[:2]:
                        raise ValueError('Fill mask size differs from atlas: ' + image.name)
            changed = False
            for face, rows, cols, positions, normals, _ in gr.islands(
                    record, uv, image.size, lambda group, s=slot: record['slots'][group[0]] == s):
                neutral = gr.neutral_mask(atlas, rows, cols, normals, record['face_normals'][face])
                if not neutral.any():
                    continue
                rows, cols, positions, normals = rows[neutral], cols[neutral], positions[neutral], normals[neutral]
                grazing = fill[rows, cols] == GRAZING_RESET if fill is not None else np.zeros(len(rows), dtype=bool)
                counts['neutral_before'] += len(rows)
                counts['grazing_reset'] = counts.get('grazing_reset', 0) + int(grazing.sum())
                best = np.full(len(rows), -np.inf)
                colors = np.zeros((len(rows), 3), dtype=np.uint8)
                for view, depth in zip(views, depths):
                    score = normals @ view.toward
                    candidate = (score > VIEW_MIN_COSINE) & (score > best)
                    if not candidate.any():
                        continue
                    index = np.flatnonzero(candidate)
                    seen, x, y = visible(view, depth, positions[index])
                    rgb, inside = sample(sheet, solid, view, x, y, editable, grazing[index])
                    take = seen & inside
                    colors[index[take]] = rgb[take]
                    best[index[take]] = score[index[take]]
                filled = np.isfinite(best)
                counts['generated'] += int(filled.sum())
                counts['unseen'] += int((~filled).sum())
                counts['grazing_unfilled'] = counts.get('grazing_unfilled', 0) + int((~filled & grazing).sum())
                if filled.any():
                    atlas[rows[filled], cols[filled], :3] = colors[filled]
                    atlas[rows[filled], cols[filled], 3] = 255
                    changed = True
            if changed:
                # Only previously neutral (gray, opaque) texels may differ.
                diff = np.any(atlas != before, axis=2)
                rgb = before[..., :3].astype(np.int16)
                gray = (rgb[..., 0] == rgb[..., 1]) & (rgb[..., 1] == rgb[..., 2]) & (before[..., 3] == 255)
                if np.any(diff & ~gray):
                    raise RuntimeError('Combine changed a non-neutral texel: ' + image.name)
                gr.write_image(image, atlas)
            counts.setdefault('images', []).append({'slot': slot, 'image': image.name, 'size': list(image.size),
                                                    'sha256': hashlib.sha256(gr.read_image(image).tobytes()).hexdigest()})
        objects[obj.name] = counts
    totals = {key: sum(o.get(key, 0) for o in objects.values())
              for key in ('neutral_before', 'generated', 'unseen', 'grazing_reset', 'grazing_unfilled')}
    return {'asset_id': asset_id, 'experiment': str(experiment), 'generated_preserved_sha256': sha(preserved),
            'views_sha256': sha(experiment / 'views.json'), 'silhouette_iou': ious,
            'totals': totals, 'objects': objects}


def geometry_record(scene):
    """Hash of every working mesh's geometry, UVs and slot assignments."""
    digest = hashlib.sha256()
    for record in scene.meshes:
        obj = record['object']
        digest.update(obj.name.encode())
        digest.update(np.ascontiguousarray(record['corners']).tobytes())
        digest.update(record['slots'].tobytes())
        for layer in obj.data.uv_layers:
            data = np.empty(len(obj.data.loops) * 2, dtype=np.float32)
            layer.data.foreach_get('uv', data)
            digest.update(layer.name.encode())
            digest.update(data.tobytes())
    return digest.hexdigest()


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--worker-in', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--asset', action='append', help='Restrict to these asset IDs')
    parser.add_argument('--no-render', action='store_true')
    parser.add_argument('--fill-masks', type=Path,
                        help='global_reproject fill-mask directory (default: <worker-in dir>/global-reprojection)')
    parser.add_argument('--include-unapproved', action='store_true',
                        help='Also combine ready-for-user candidates without a texture approval (review builds only)')
    args = parser.parse_args(argv)
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
    from render_slots import acquire
    import global_reproject as gr
    output = args.output.resolve()
    if output.exists():
        raise FileExistsError(output)
    worker_in = args.worker_in.resolve(strict=True)
    worker_sha = sha(worker_in)
    selected = candidates(approved_only=not args.include_unapproved)
    if args.asset:
        missing = set(args.asset) - set(selected)
        if missing:
            raise ValueError('No ready texture candidate for: ' + ', '.join(sorted(missing)))
        selected = {key: selected[key] for key in args.asset}
    fill_masks = (args.fill_masks or worker_in.parent / 'global-reprojection').resolve(strict=True)
    acquire()
    import bpy
    bpy.ops.wm.open_mainfile(filepath=str(worker_in))
    bpy.context.window.scene = bpy.data.scenes[gr.SCENE]
    scene = gr.Scene()
    geometry_before = geometry_record(scene)
    output.mkdir(parents=True)
    assets, failures = [], {}
    for asset_id, experiment in sorted(selected.items()):
        try:
            assets.append(combine_asset(asset_id, experiment, scene, scene.meshes, gr,
                                        fill_masks))
            print(json.dumps({'asset': asset_id, **assets[-1]['totals']}), flush=True)
        except Exception as error:
            failures[asset_id] = f'{type(error).__name__}: {error}'
            print(json.dumps({'asset': asset_id, 'failed': failures[asset_id]}), flush=True)
    if geometry_record(gr.Scene()) != geometry_before:
        raise RuntimeError('Combine changed geometry, UVs or slot assignments')
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'))
    if not args.no_render:
        from render_multiview_asset import render
        from refinement_review import _tile
        from array import array
        for row in assets:
            directory = output / 'renders' / row['asset_id']
            manifest = Path(row['experiment']) / 'views.json'
            width, height = json.loads(manifest.read_text())['tile_size']
            render(manifest, directory, width=width)
            buffers = []
            for index in range(8):
                image = bpy.data.images.load(str(directory / f'view-{index}-textured.png'), check_existing=False)
                buffer = array('f', [0]) * len(image.pixels)
                image.pixels.foreach_get(buffer)
                buffers.append(buffer)
                bpy.data.images.remove(image)
            _tile(buffers, width, height, directory / 'textured.png')
            row['actual_sheet_sha256'] = sha(directory / 'textured.png')
    report = {'version': 1, 'fill_masks': str(fill_masks), 'worker_in': str(worker_in), 'texture_approved_only': not args.include_unapproved, 'worker_in_sha256': worker_sha,
              'worker_out_sha256': sha(output / 'worker.blend'), 'geometry_sha256': geometry_before,
              'rule': 'Only texels matching global_reproject.neutral_mask are written; best-facing first-hit view of the asset\'s own meshes; generated-preserved.png sampled inside the reviewed silhouette.',
              'assets': assets, 'failures': failures}
    (output / 'combine.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'output': str(output), 'assets': len(assets), 'failures': len(failures)}), flush=True)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])
