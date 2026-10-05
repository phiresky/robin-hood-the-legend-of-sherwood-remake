"""Expand a private stump volume to retain all explicitly observed wood pixels."""
import json
import math
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image
sys.path.insert(0, str(Path(__file__).parent))
from restart2_tree18 import OUT, SIN, COS
from render_slots import acquire
from refinement_workspace import prepare, modified, validate
from refinement_inventory import inventory
from evidence_io import sha


def main():
    dest = OUT / 'restart2/stump68-wood-fit-v2'
    dest.mkdir(exist_ok=False)
    original = OUT / 'stump68-wood-review-v1/assets/croisement01-southeast-small-stump'
    domain = OUT / 'stump68-wood-review-v1/source/wood-domain.png'
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(original / 'model.blend'))
    bpy.context.preferences.filepaths.save_version = 0
    config = json.loads((original / 'workspace.json').read_text())
    collection = bpy.data.collections[config['collection_name']]
    obj = next(o for o in collection.all_objects if o.get('source_node') == 'building-059' and o.type == 'MESH')
    mesh = obj.data
    alpha = np.asarray(Image.open(domain).convert('L')) > 127
    points = np.asarray([tuple(obj.matrix_world @ v.co) for v in mesh.vertices])
    projected = -points[:, 1] * SIN - points[:, 2] * COS
    observed_top = 635 + np.where(alpha)[0].min()
    old_top = float(projected.min())
    inverse = obj.matrix_world.inverted()
    top_expansion = max(0., old_top - observed_top + .25)
    for vertex, point, y in zip(mesh.vertices, points, projected):
        weight = max(0., 1. - (y - old_top) / 12.)
        vertex.co = inverse @ Vector((point[0], point[1], point[2] + top_expansion * weight / COS))
    mesh.update()
    points = np.asarray([tuple(obj.matrix_world @ v.co) for v in mesh.vertices])
    projected = np.column_stack([points[:, 0], -points[:, 1] * SIN - points[:, 2] * COS])
    edges = np.asarray([tuple(edge.vertices) for edge in mesh.edges])
    a, b = projected[edges[:, 0]], projected[edges[:, 1]]
    dy = b[:, 1] - a[:, 1]
    samples, old_left, old_right, left, right = [], [], [], [], []
    for row in range(alpha.shape[0]):
        xs = np.flatnonzero(alpha[row])
        y = 635 + row + .5
        active = (np.minimum(a[:, 1], b[:, 1]) <= y) & (np.maximum(a[:, 1], b[:, 1]) >= y) & (np.abs(dy) > 1e-8)
        if not len(xs) or not active.any():
            continue
        intersections = a[active, 0] + (y - a[active, 1]) / dy[active] * (b[active, 0] - a[active, 0])
        lo, hi = float(intersections.min()), float(intersections.max())
        samples.append(y)
        old_left.append(lo)
        old_right.append(hi)
        # Unobserved flanks retain their existing inferred volume; partial bark
        # behind foreground grass must not collapse the hidden stump diameter.
        left.append(min(lo, 894 + xs[0] - .3))
        right.append(max(hi, 894 + xs[-1] + 1.3))
    if len(samples) < 20:
        raise ValueError('Insufficient observed wood rows for contour refinement')
    changes = []
    for vertex, point, uv in zip(mesh.vertices, points, projected):
        y = uv[1]
        lo, hi = np.interp(y, samples, old_left), np.interp(y, samples, old_right)
        if hi - lo < .01:
            continue
        lower, upper = np.interp(y, samples, left), np.interp(y, samples, right)
        target = lower + (point[0] - lo) * (upper - lower) / (hi - lo)
        weight = min(1., max(0., (samples[-1] + 8 - y) / 8.))
        dx = (target - point[0]) * weight
        if abs(dx) > 12:
            raise ValueError('Contour correction exceeds source-supported stump scale')
        vertex.co = inverse @ Vector((point[0] + dx, point[1], point[2]))
        changes.append(float(dx))
    mesh.update()
    (dest / 'construction.json').write_text(json.dumps(dict(status='private candidate; actual/source/contact review required',
        source_model_sha256=sha(original / 'model.blend'), observed_wood_sha256=sha(domain),
        method='Expand full-volume source-camera contour only where observed wood extends beyond it; retain hidden flank depth and diameter.',
        top_expansion=top_expansion, horizontal_change=[min(changes), max(changes)],
        deferred_native_pixels=786, limitations=['Foreground foliage remains a separate unresolved source domain.']), indent=2) + '\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(dest / 'input.blend'))
    inventory(dest / 'inventory', collection_name=collection.name, map_name='Croisement01',
              source_path=OUT / 'baseline/covered.png', patch_manifest=OUT / 'source-states/layers.json')
    grouping = original / 'reference/grouping.json'
    review = dest / 'grouping-review.json'
    review.write_text(json.dumps(dict(status='reviewed', reviewer='Codex', catalog_sha256=sha(grouping),
        inventory_sha256=sha(dest / 'inventory/inventory.json'),
        evidence='Unchanged native mask68/source part59 wood ownership. Only the explicitly observed cap and bark silhouette are refined; grass is deferred.'), indent=2) + '\n')
    worker = dest / 'assets' / config['asset_id']
    prepare(worker, asset_id=config['asset_id'], scene_name=config['scene_name'], collection_name=collection.name,
            source_path=OUT / 'baseline/covered.png', grouping_manifest=grouping,
            inventory_path=dest / 'inventory/inventory.json', review_path=review,
            source_mask_manifest=original / 'source-masks.json', width=384, height=384,
            framing_padding=1.2, lighting=dict(toward_sun=[-.6, -.4, .7], ambient=.22, diffuse=.78, shadow_epsilon=.05))
    validate(worker)
    modified(worker)
    (worker / 'inspection').mkdir(exist_ok=True)
    import render_candidate
    sys.argv = ['render', '--', str(worker)]
    render_candidate.main()


if __name__ == '__main__':
    main()
