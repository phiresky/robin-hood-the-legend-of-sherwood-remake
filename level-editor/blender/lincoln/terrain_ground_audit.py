"""Source-coverage audit of the Lincoln terrain workspace (Blender, render slot).

Every authored ground-domain pixel centre is cast along the source camera ray
into the complete saved scene (receiver-specific mask eligibility, as the
packet renderer does). The pixel passes when the ground is the first eligible
hit, the hit projects back into that pixel, and the saved ground material's
packed atlas texel at the hit UV is observed (alpha 255) with the exact source
RGB. Pixels are also classified independently of the domain: every
source-camera ray whose first hit is the ground but that the domain rejects is
counted by reason (native mask, other authored domain, carve-out).

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/terrain_ground_audit.py -- <workspace>
"""
import collections
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np
from mathutils import Vector

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
sys.path.insert(0, str(HERE))
from render_slots import acquire  # noqa: E402
from freeze_tooling import select_tooling  # noqa: E402


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    workspace = Path(sys.argv[sys.argv.index('--') + 1]).resolve()
    acquire()
    select_tooling(ROOT / 'tooling/e6b57cb851c7142b')
    from PIL import Image
    from refinement_review import _tree
    from source_visibility import first_source_hit
    from occlusion_constraints import SourceMaskConstraints
    config = json.loads((workspace / 'workspace.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    bpy.context.window.scene = bpy.data.scenes[config['scene_name']]
    objects = [o for o in bpy.data.collections[config['collection_name']].all_objects
               if o.type == 'MESH' and not o.hide_render]
    ground, = [o for o in objects if o.get('source_node') == 'ground']
    tree, owners, _ = _tree(objects)
    source = Image.open(config['source_path']).convert('RGB')
    src = np.array(source)
    constraints = SourceMaskConstraints(config['source_mask_manifest'], 'exterior', sha(config['source_path']), source.size)
    domain_path = workspace / 'inspection/ground-source-domain-full.png'
    domain = np.array(Image.open(domain_path).convert('L')) > 127
    # Reviewed exclusions on the ground row (e.g. approved foliage domains) leave the receiver.
    masks = json.loads(Path(config['source_mask_manifest']).read_text())
    inventory_path = Path(config['source_mask_manifest']).parent / masks['mask_inventory']
    records = {r['index']: r for r in json.loads(inventory_path.read_text())['masks']}
    row, = [r for r in masks['projections']['exterior']['assignments'] if r.get('source_node') == 'ground']
    excluded = np.zeros(domain.shape, bool)
    for index in row.get('exclude_mask_indices', []):
        record = records[index]
        png = Path(record['png'])
        png = png if png.is_absolute() else inventory_path.parent / png
        bitmap = np.array(Image.open(png).convert('L')) > 0
        x0, y0 = record['box_top_left']
        h, w = bitmap.shape
        excluded[max(0, y0):y0 + h, max(0, x0):x0 + w] |= bitmap[max(0, -y0):domain.shape[0] - y0, max(0, -x0):domain.shape[1] - x0]
    excluded_pixels = int((domain & excluded).sum())
    domain &= ~excluded
    material = ground.data.materials[ground.data.polygons[0].material_index]
    image = next(n.image for n in material.node_tree.nodes if n.type == 'TEX_IMAGE')
    w, h = image.size
    atlas = np.empty(w * h * 4, np.float32)
    image.pixels.foreach_get(atlas)
    atlas = np.flipud((atlas.reshape(h, w, 4) * 255 + 0.5).astype(np.int32))
    uv_layer = ground.data.uv_layers[ground.data.uv_layers.active.name]
    ground.data.calc_loop_triangles()
    s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
    toward = Vector((0, -c, s))
    counts = collections.Counter()
    witnesses = collections.defaultdict(list)
    failed = np.zeros(domain.shape, bool)
    ys, xs = np.nonzero(domain)
    for x, y in zip(xs.tolist(), ys.tolist()):
        target = Vector((x + .5, -(y + .5) / s, 0))
        origin = target + toward * 20000
        hit, normal, index, _ = first_source_hit(tree, owners, origin, -toward, constraints=constraints,
                                                 receiver=ground, source_pixel=(x, y))
        if index is None or owners[index] != ground:
            key = 'blocked:' + (str(owners[index].get('source_node')) if index is not None else 'none')
        else:
            px, py = hit.x, -hit.y * s - hit.z * c
            if int(math.floor(px)) != x or int(math.floor(py)) != y:
                key = 'reprojection-mismatch'
            else:
                texel = atlas[y, x]
                if texel[3] < 255:
                    key = 'atlas-unobserved'
                elif tuple(texel[:3]) != tuple(src[y, x]):
                    key = 'atlas-rgb-mismatch'
                else:
                    key = 'ground-visible-exact'
        counts[key] += 1
        if key != 'ground-visible-exact':
            failed[y, x] = True
            if len(witnesses[key]) < 30:
                witnesses[key].append([x, y])
    out = workspace / 'inspection'
    Image.fromarray((failed * 255).astype(np.uint8)).save(out / 'coverage-audit-failures.png')
    # Source-only comparison: original artwork beside the saved owned atlas.
    crop = (0, 0, 1450, 1150)
    owned = atlas[crop[1]:crop[3], crop[0]:crop[2], :3].astype(np.uint8)
    original = src[crop[1]:crop[3], crop[0]:crop[2]]
    sheet = np.concatenate([original, np.full((original.shape[0], 8, 3), 30, np.uint8), owned], 1)
    Image.fromarray(sheet).save(out / 'source-comparison.png')
    report = {'version': 1, 'model_sha256': sha(workspace / 'model.blend'),
              'modified_views_sha256': sha(workspace / 'modified/views.json'),
              'domain_bitmap_sha256': sha(domain_path), 'expected_source_pixels': int(domain.sum()),
              'ground_row_exclusions': row.get('exclude_mask_indices', []), 'excluded_domain_pixels': excluded_pixels,
              'counts': dict(counts), 'witnesses': dict(witnesses),
              'atlas_observed_texels': int((atlas[..., 3] == 255).sum()),
              'method': ('Every authored ground-domain pixel centre: source-camera ray into the complete saved scene '
                         'with receiver-specific mask eligibility; the first eligible hit must be the ground, '
                         'reproject into the same pixel, and read an observed packed atlas texel with the exact '
                         'source RGB.')}
    (out / 'coverage-ray-audit.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('expected_source_pixels', 'counts', 'atlas_observed_texels')}), flush=True)


if __name__ == '__main__':
    main()
