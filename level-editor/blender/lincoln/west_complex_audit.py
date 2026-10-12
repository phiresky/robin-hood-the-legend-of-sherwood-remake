"""Source-coverage audit for a Lincoln west-complex workspace (PROCEDURE section 6).

Run in Blender on a workspace after its modified packet exists:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/west_complex_audit.py -- --asset <id>

The audit domain is derived independently of the acceptance masks: every source
pixel is ray cast from the 35-degree source camera through the saved model
(complete scene context, render-visible meshes). Each pixel whose first hit is
an owned mesh, or which lies inside an owned node's reviewed native mask, is
classified:

- accepted: first hit owned node and inside that node's reviewed mask;
- neutral-reject-all: first hit owned node whose constraint is reject-all;
- neutral-backfacing: first hit owned face turned away from the source camera
  (the projection never textures it; a sign of wrong orientation/slope);
- neutral-outside-mask: first hit owned node outside its mask / inside a reviewed
  foreground exclusion (geometry wider than the artwork silhouette, or foreground);
- mask-foreign-hit: inside an owned mask but first hit is another asset (a
  composite envelope delegated to a neighbour, or a foreground occluder);
- mask-no-geometry: inside an owned mask but the ray hits nothing (missing
  geometry candidates).

It writes inspection/coverage/coverage.json and an annotated evidence image,
plus a source-camera render of the saved materials beside the artwork. The
worker then writes source-coverage-audit.json with the observations.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
REFINEMENT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
# Workspace round: round-1 (default) or round-2 (integrated context); set with
# --round or the WEST_COMPLEX_ROUND environment variable.
import os as _os
ROUND = int(_os.environ.get('WEST_COMPLEX_ROUND', '1'))
ASSETS_DIR = REFINEMENT / f'round-{ROUND}/assets'
S, C = math.sin(math.radians(35)), math.cos(math.radians(35))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def node_masks(workspace, nodes, size):
    from PIL import Image
    masks = json.loads((workspace / 'source-masks.json').read_text())
    inventory_dir = REFINEMENT / 'mask-review/inventory-v1'
    manifest = {m['index']: m for m in json.loads((inventory_dir / 'manifest.json').read_text())['masks']}
    assignments = {a['source_node']: a for a in masks['projections']['exterior']['assignments']}
    cache = {}

    def full(index):
        if index not in cache:
            record = manifest[index]
            image = Image.new('L', size, 0)
            tile = Image.open(inventory_dir / record['png']).convert('L').point(lambda v: 255 if v > 127 else 0)
            image.paste(tile, tuple(record['box_top_left']))
            cache[index] = image
        return cache[index]
    result = {}
    for node in nodes:
        a = assignments[node]
        include = Image.new('L', size, 0)
        for index in a['mask_indices']:
            include.paste(255, (0, 0), full(index))
        for index in a.get('exclude_mask_indices') or []:
            include.paste(0, (0, 0), full(index))
        result[node] = {'image': include, 'reject_all': a['constraint_kind'] == 'unknown-no-approved-source',
                        'group': a.get('review_group'), 'masks': a['mask_indices'],
                        'excludes': a.get('exclude_mask_indices') or []}
    return result


def audit(asset, combined=False):
    import bpy
    from mathutils import Vector
    from mathutils.bvhtree import BVHTree
    from PIL import Image, ImageDraw
    workspace = ASSETS_DIR / asset
    config = json.loads((workspace / 'workspace.json').read_text())
    if not combined:
        bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    source = Image.open(config['source_path']).convert('RGB')
    size = source.size
    depsgraph = bpy.context.evaluated_depsgraph_get()
    objects = [o for o in bpy.data.collections[config['collection_name']].all_objects
               if o.type == 'MESH' and not o.hide_render]
    verts, tris, owner = [], [], []
    for obj in objects:
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        mesh.calc_loop_triangles()
        base = len(verts)
        verts.extend(obj.matrix_world @ v.co for v in mesh.vertices)
        for tri in mesh.loop_triangles:
            tris.append([base + i for i in tri.vertices])
            owner.append(obj)
        evaluated.to_mesh_clear()
    tree = BVHTree.FromPolygons(verts, tris, all_triangles=True)
    owned = [o for o in objects if o.get('asset_group') == asset]
    nodes = sorted({o['source_node'] for o in owned})
    masks = node_masks(workspace, nodes, size)
    xs, ys = [], []
    for obj in owned:
        for v in obj.data.vertices:
            w = obj.matrix_world @ v.co
            xs.append(w.x)
            ys.append(-w.y * S - w.z * C)
    for node in nodes:
        box = masks[node]['image'].getbbox()
        if box:
            xs += [box[0], box[2]]
            ys += [box[1], box[3]]
    x0, x1 = max(0, int(min(xs)) - 3), min(size[0], int(max(xs)) + 4)
    y0, y1 = max(0, int(min(ys)) - 3), min(size[1], int(max(ys)) + 4)
    toward = Vector((0, -C, S))
    mask_px = {n: masks[n]['image'].load() for n in nodes}
    counts = {k: 0 for k in ('accepted', 'neutral-reject-all', 'neutral-backfacing', 'neutral-outside-mask',
                             'mask-foreign-hit', 'mask-no-geometry')}
    per_node = {n: {k: 0 for k in counts} for n in nodes}
    foreign = {}
    colours = {'accepted': (40, 220, 60), 'neutral-reject-all': (150, 150, 150),
               'neutral-backfacing': (200, 0, 200),
               'neutral-outside-mask': (255, 40, 40), 'mask-foreign-hit': (40, 120, 255),
               'mask-no-geometry': (255, 230, 0)}
    overlay = source.crop((x0, y0, x1, y1)).copy()
    opx = overlay.load()
    for py in range(y0, y1):
        for px in range(x0, x1):
            u, v = px + .5, py + .5
            origin = Vector((u, -v / S, 0)) + toward * 20000
            hit, normal, index, _ = tree.ray_cast(origin, -toward)
            obj = owner[index] if hit is not None else None
            inside = [n for n in nodes if mask_px[n][px, py]]
            category = node = None
            if obj is not None and obj.get('asset_group') == asset:
                node = obj['source_node']
                if masks[node]['reject_all']:
                    category = 'neutral-reject-all'
                elif normal.dot(toward) <= 1e-4:
                    category = 'neutral-backfacing'
                elif mask_px[node][px, py]:
                    category = 'accepted'
                else:
                    category = 'neutral-outside-mask'
            elif inside:
                node = inside[0]
                if obj is None:
                    category = 'mask-no-geometry'
                else:
                    category = 'mask-foreign-hit'
                    key = obj.get('asset_group') or obj.get('source_node') or obj.name
                    foreign[key] = foreign.get(key, 0) + 1
            if category:
                counts[category] += 1
                per_node[node][category] += 1
                r, g, b = opx[px - x0, py - y0]
                c = colours[category]
                opx[px - x0, py - y0] = ((r + 2 * c[0]) // 3, (g + 2 * c[1]) // 3, (b + 2 * c[2]) // 3)
    out = workspace / ('inspection/coverage-combined' if combined else 'inspection/coverage')
    out.mkdir(parents=True, exist_ok=True)
    scale = max(1, min(4, int(1400 / max(x1 - x0, y1 - y0))))
    plain = source.crop((x0, y0, x1, y1)).resize(((x1 - x0) * scale, (y1 - y0) * scale), Image.NEAREST)
    marked = overlay.resize(plain.size, Image.NEAREST)
    sheet = Image.new('RGB', (plain.width * 2 + 8, plain.height + 30), (0, 0, 0))
    sheet.paste(plain, (0, 30))
    sheet.paste(marked, (plain.width + 8, 30))
    draw = ImageDraw.Draw(sheet)
    x = 4
    for name, c in colours.items():
        draw.rectangle((x, 8, x + 12, 20), fill=c)
        draw.text((x + 16, 8), name, fill=(255, 255, 255))
        x += 20 + 7 * len(name)
    sheet.save(out / 'coverage.png')
    domain = (counts['accepted'] + counts['neutral-reject-all'] + counts['neutral-backfacing']
              + counts['neutral-outside-mask'])
    mask_total = counts['accepted'] + counts['mask-foreign-hit'] + counts['mask-no-geometry']
    report = {'version': 1, 'asset_id': asset, 'crop': [x0, y0, x1, y1],
              'scene': ('combined west-complex scene: every lane workspace\'s refined owned meshes swapped into one '
                        'file (scratch/west_complex/combined_scene.py); neighbours outside the lane stay baseline')
                       if combined else 'isolated workspace model.blend (neighbours at baseline)',
              'model_sha256': sha(workspace / 'model.blend'),
              'modified_views_sha256': sha(workspace / 'modified/views.json'),
              'source_sha256': sha(config['source_path']),
              'counts': counts, 'per_node': per_node, 'foreign_first_hits': dict(sorted(foreign.items(), key=lambda kv: -kv[1])),
              'geometry_visible_domain_pixels': domain, 'owned_mask_pixels': mask_total,
              'accepted_fraction_of_visible_domain': counts['accepted'] / max(1, domain),
              'accepted_fraction_of_mask': counts['accepted'] / max(1, mask_total),
              'node_constraints': {n: {k: v for k, v in masks[n].items() if k != 'image'} for n in nodes},
              'evidence': {'coverage.png': sha(out / 'coverage.png')}}
    (out / 'coverage.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('counts', 'foreign_first_hits', 'accepted_fraction_of_visible_domain',
                                              'accepted_fraction_of_mask')}, indent=2))


LANE = ['lincoln-west-round-tower', 'lincoln-west-tower-hall', 'lincoln-west-slate-tower',
        'lincoln-west-tower-terrace', 'lincoln-west-upper-curtain-wall', 'lincoln-garden-north-wall',
        'lincoln-garden', 'lincoln-garden-fountain', 'lincoln-inner-west-gate']


def load_combined(base_asset):
    """Open one workspace and swap in every lane workspace's owned meshes (inspection only)."""
    import bpy
    bpy.ops.wm.open_mainfile(filepath=str(ASSETS_DIR / base_asset / 'model.blend'))
    collection = bpy.data.collections['lincoln Working']
    swapped = {}
    for asset in LANE:
        if asset == base_asset:
            continue
        path = ASSETS_DIR / asset / 'model.blend'
        targets = {o.name: o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == asset}
        with bpy.data.libraries.load(str(path)) as (src, dst):
            dst.objects = [name for name in src.objects if name in targets]
        for loaded in dst.objects:
            if loaded is None:
                continue
            target = targets[loaded.name.rsplit('.', 1)[0] if loaded.name not in targets else loaded.name]
            target.data = loaded.data
            bpy.data.objects.remove(loaded)
            swapped[asset] = swapped.get(asset, 0) + 1
    print('combined swap', swapped)
    return swapped


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument('--round', type=int, default=None)
    parser.add_argument('--asset', required=True)
    parser.add_argument('--combined', action='store_true',
                        help='also audit against the other lane assets refined meshes (inspection only)')
    args = parser.parse_args(argv)
    if args.round is not None:
        global ASSETS_DIR
        ASSETS_DIR = ASSETS_DIR.parents[1] / f'round-{args.round}/assets'
    from render_slots import acquire
    acquire()
    if args.combined:
        load_combined(args.asset)
    audit(args.asset, combined=args.combined)


if __name__ == '__main__':
    main()
