"""Source-coverage audit for a Lincoln village workspace (PROCEDURE section 6).

Reopens the saved ``model.blend`` and casts one source-camera ray per artwork pixel
around the asset (orthographic 35 degree camera: pixel (x, y) <-> world ray
X = x, -Y sin35 - Z cos35 = y).  The first hit over every render-visible working
mesh, including the terrain ground and all neighbouring assets, gives the
source-visible domain of the asset independently of the acceptance masks. That
domain is then compared with the reviewed native masks from the workspace's
``source-masks.json``:

* accepted  = owned first hit inside the reviewed mask (minus reviewed exclusions)
* rejected  = owned first hit outside the mask, or on a reject-all node
* mask-only = mask pixels whose first hit is ground, another asset, or nothing

Writes ``inspection/source-coverage.png`` (unmarked crop | classified overlay |
actual saved-model source view) and ``inspection/source-coverage-domain.json``.
``source-coverage-audit.json`` itself is written by the worker after reviewing
this evidence together with the modified packet.

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/village_audit.py -- --asset <asset-id>
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
ROOT = REPO / 'level-editor/work/needs-review/lincoln-refinement'
sys.path.insert(0, str(HERE))

import bpy  # noqa: E402
import numpy as np  # noqa: E402
from mathutils import Vector  # noqa: E402
from mathutils.bvhtree import BVHTree  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402

S = math.sin(math.radians(35))
C = math.cos(math.radians(35))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def masks_for(workspace, nodes):
    doc = json.loads((workspace / 'source-masks.json').read_text())
    assignments = {e['source_node']: e for e in doc['projections']['exterior']['assignments']}
    manifest = Path(doc['mask_inventory'])
    if not manifest.is_absolute():
        manifest = ROOT / 'mask-review' / manifest
    inventory = json.loads(manifest.read_text())['masks']
    inv_dir = manifest.parent
    result = {}
    for node in nodes:
        entry = assignments[node]
        include = np.zeros((2176, 2944), bool)
        for index in entry['mask_indices']:
            if index == 428:
                continue
            m = inventory[index]
            bx, by = m['box_top_left']
            bitmap = np.array(Image.open(inv_dir / m['png']).convert('L')) > 0
            h, w = bitmap.shape
            include[by:by + h, bx:bx + w] |= bitmap
        for index in entry.get('exclude_mask_indices', []):
            m = inventory[index]
            bx, by = m['box_top_left']
            bitmap = np.array(Image.open(inv_dir / m['png']).convert('L')) > 0
            h, w = bitmap.shape
            include[by:by + h, bx:bx + w] &= ~bitmap
        result[node] = (include, entry)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--asset', required=True)
    parser.add_argument('--round', default='round-1', help='workspace round directory, e.g. round-2')
    parser.add_argument('--pad', type=int, default=16)
    parser.add_argument('--terrain-blend', help='preview only: swap in the ground mesh from this blend (not saved)')
    parser.add_argument('--rebuild', action='store_true', help='preview only: rebuild owned meshes from the current recipe (not saved)')
    parser.add_argument('--out-dir', help='preview only: write evidence here instead of the workspace inspection/')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = ROOT / args.round / 'assets' / args.asset
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    config = json.loads((workspace / 'workspace.json').read_text())
    working = bpy.data.collections[config['collection_name']]
    preview = bool(args.terrain_blend or args.rebuild)
    if preview and not args.out_dir:
        raise ValueError('Preview modes must write to --out-dir, never the frozen workspace evidence')
    if args.terrain_blend:
        with bpy.data.libraries.load(args.terrain_blend) as (src, dst):
            dst.objects = [n for n in src.objects if n == 'lincoln Terrain']
        new = dst.objects[0]
        old = working.all_objects['lincoln Terrain']
        old.data = new.data.copy()
        old.matrix_world = new.matrix_world.copy()
    if args.rebuild:
        import refine_village
        refine_village.refine(args.asset)
        bpy.context.view_layer.update()
    depsgraph = bpy.context.evaluated_depsgraph_get()
    verts, tris, owner = [], [], []
    names = []
    owned_bounds = []
    exclusion = config.get('source_projection_ground_exclusion')
    excluded = exclusion['object_name'] if exclusion else None
    for obj in working.all_objects:
        if obj.type != 'MESH' or obj.hide_render or obj.name == excluded:
            continue
        ev = obj.evaluated_get(depsgraph)
        mesh = ev.to_mesh()
        mesh.calc_loop_triangles()
        off = len(verts)
        world = [obj.matrix_world @ v.co for v in mesh.vertices]
        verts.extend(world)
        idx = len(names)
        names.append(obj)
        for t in mesh.loop_triangles:
            tris.append(tuple(off + i for i in t.vertices))
            owner.append(idx)
        if obj.get('asset_group') == args.asset:
            owned_bounds.extend((p.x, -p.y * S - p.z * C) for p in world)
        ev.to_mesh_clear()
    tree = BVHTree.FromPolygons(verts, tris, all_triangles=True)
    xs = [p[0] for p in owned_bounds]
    ys = [p[1] for p in owned_bounds]
    x0 = max(int(math.floor(min(xs))) - args.pad, 0)
    y0 = max(int(math.floor(min(ys))) - args.pad, 0)
    x1 = min(int(math.ceil(max(xs))) + args.pad, 2944)
    y1 = min(int(math.ceil(max(ys))) + args.pad, 2176)
    direction = Vector((0.0, C, -S))
    top = 3000.0
    hit_owner = np.full((y1 - y0, x1 - x0), -1, np.int32)
    for py in range(y0, y1):
        for px in range(x0, x1):
            y = py + 0.5
            origin = Vector((px + 0.5, -(y + top * C) / S, top))
            loc, normal, index, dist = tree.ray_cast(origin, direction)
            if index is not None:
                hit_owner[py - y0, px - x0] = owner[index]
    owned_idx = {i for i, o in enumerate(names) if o.get('asset_group') == args.asset}
    out_dir_early = Path(args.out_dir) if args.out_dir else workspace / 'inspection'
    out_dir_early.mkdir(parents=True, exist_ok=True)
    node_of = {i: names[i].get('source_node') for i in owned_idx}
    nodes = sorted(set(node_of.values()))
    masks = masks_for(workspace, nodes)
    union = np.zeros((y1 - y0, x1 - x0), bool)
    for node in nodes:
        union |= masks[node][0][y0:y1, x0:x1]
    accepted = np.zeros_like(union)
    rejected_outside = np.zeros_like(union)
    rejected_reject_all = np.zeros_like(union)
    per_node = {}
    for i in owned_idx:
        node = node_of[i]
        hit = hit_owner == i
        mask, entry = masks[node]
        m = mask[y0:y1, x0:x1]
        reject_all = entry['constraint_kind'] == 'unknown-no-approved-source'
        a = hit & m if not reject_all else np.zeros_like(hit)
        accepted |= a
        if reject_all:
            rejected_reject_all |= hit
        else:
            rejected_outside |= hit & ~m
        if preview and not reject_all and (hit & ~m).any():
            rej = out_dir_early / f'rejected-outside-mask-{node}.png'
            Image.fromarray(((hit & ~m) * 255).astype(np.uint8)).save(rej)
        rec = per_node.setdefault(node, {'first_hit_pixels': 0, 'accepted': 0, 'rejected_outside_mask': 0,
                                         'reject_all': reject_all, 'mask_indices': entry['mask_indices'],
                                         'exclude_mask_indices': entry.get('exclude_mask_indices', [])})
        rec['first_hit_pixels'] += int(hit.sum())
        rec['accepted'] += int(a.sum())
        rec['rejected_outside_mask'] += int((hit & ~m).sum()) if not reject_all else int(hit.sum())
    owned_any = np.isin(hit_owner, list(owned_idx))
    mask_only = union & ~owned_any
    ground_idx = {i for i, o in enumerate(names)
                  if o.name.startswith('lincoln Terrain') or o.get('asset_group') == 'lincoln Terrain'}
    mask_only_ground = mask_only & np.isin(hit_owner, list(ground_idx))
    mask_only_nothing = mask_only & (hit_owner < 0)
    mask_only_other = mask_only & ~mask_only_ground & ~mask_only_nothing
    other_owners = {}
    for i in np.unique(hit_owner[mask_only_other]):
        other_owners[names[i].get('asset_group') or names[i].name] = other_owners.get(
            names[i].get('asset_group') or names[i].name, 0) + int((hit_owner[mask_only_other] == i).sum())
    source = np.array(Image.open(config['source_path']).convert('RGB'))[y0:y1, x0:x1].astype(float)
    over = source.copy()
    for arr, colour in ((accepted, (0, 220, 0)), (rejected_outside, (255, 40, 40)),
                        (rejected_reject_all, (255, 150, 0)), (mask_only_ground, (40, 110, 255)),
                        (mask_only_nothing, (40, 110, 255)), (mask_only_other, (230, 230, 0))):
        over[arr] = over[arr] * 0.45 + np.array(colour) * 0.55
    actual = np.full_like(source, 110.0)
    actual[accepted] = source[accepted]
    scale = 3 if (x1 - x0) < 300 else 2
    tiles = [Image.fromarray(a.astype(np.uint8)) for a in (source, over, actual)]
    w, h = tiles[0].size
    sheet = Image.new('RGB', (w * 3 * scale + 8, h * scale + 16), (0, 0, 0))
    for k, t in enumerate(tiles):
        sheet.paste(t.resize((w * scale, h * scale), Image.NEAREST), (k * (w * scale + 4), 16))
    d = ImageDraw.Draw(sheet)
    d.text((2, 2), f'{args.asset} crop {x0},{y0} | source | green accepted, red owned-outside-mask, orange reject-all, '
                   f'blue mask-only ground/none, yellow mask-only other | saved model source view', fill=(255, 255, 255))
    out_dir = Path(args.out_dir) if args.out_dir else workspace / 'inspection'
    out_dir.mkdir(parents=True, exist_ok=True)
    # Authored-domain proposals for reject-all receivers: the first-hit domain of
    # each such node, as a full-resolution-coordinate bitmap and column polygon.
    reject_domains = {}
    for node in nodes:
        if masks[node][1]['constraint_kind'] != 'unknown-no-approved-source':
            continue
        dom = np.isin(hit_owner, [i for i in owned_idx if node_of[i] == node])
        if not dom.any():
            continue
        ys_, xs_ = np.where(dom)
        top, bottom = [], []
        for cx in range(xs_.min(), xs_.max() + 1):
            col = np.where(dom[:, cx])[0]
            if len(col):
                top.append([int(cx + x0), int(col.min() + y0)])
                bottom.append([int(cx + x0 + 1), int(col.max() + y0 + 1)])
        bitmap = out_dir / f'reject-all-domain-{node}.png'
        Image.fromarray((dom * 255).astype(np.uint8)).save(bitmap)
        reject_domains[node] = {'pixels': int(dom.sum()), 'bitmap': str(bitmap), 'bitmap_sha256': sha(bitmap),
                                'bitmap_box_top_left': [x0, y0], 'bitmap_size': [x1 - x0, y1 - y0],
                                'column_polygon_source_px': top + bottom[::-1]}
    png = out_dir / 'source-coverage.png'
    sheet.save(png)
    report = {'version': 1, 'asset_id': args.asset, 'preview': preview, 'terrain_blend': args.terrain_blend,
              'rebuilt_from_recipe': args.rebuild, 'model_sha256': sha(workspace / 'model.blend'),
              'modified_views_sha256': sha(workspace / 'modified/views.json'),
              'source_sha256': sha(config['source_path']),
              'crop': [x0, y0, x1, y1],
              'method': 'Per-pixel first-hit ray cast from the source camera over all render-visible working meshes '
                        '(including ground and neighbours) in the saved model; compared with reviewed masks.',
              'owned_first_hit_pixels': int(owned_any.sum()),
              'accepted_pixels': int(accepted.sum()),
              'rejected_outside_mask_pixels': int(rejected_outside.sum()),
              'rejected_reject_all_pixels': int(rejected_reject_all.sum()),
              'mask_only_pixels': int(mask_only.sum()),
              'mask_only_ground_pixels': int(mask_only_ground.sum()),
              'mask_only_no_hit_pixels': int(mask_only_nothing.sum()),
              'mask_only_other_object_pixels': int(mask_only_other.sum()),
              'mask_only_other_owners': other_owners,
              'mask_union_pixels': int(union.sum()),
              'per_node': per_node,
              'ground_exclusion': excluded,
              'reject_all_first_hit_domains': reject_domains,
              'evidence': {str(png): sha(png)}}
    (out_dir / 'source-coverage-domain.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k not in ('per_node',)}))


if __name__ == '__main__':
    main()
