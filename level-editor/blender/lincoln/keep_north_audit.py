"""Source-camera coverage audit of a saved keep_north workspace model.

  /usr/bin/blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/keep_north_audit.py -- --asset <id>

Ray-casts the frozen 35-degree source camera through every pixel of the asset's
context crop against all visible working meshes.  The audit domain is every
pixel whose first hit is an owned mesh; it is derived from geometry, not from
the acceptance masks.  Each domain pixel is then classified by the working
native-mask rule (union of include masks minus exclusions) and by whether the
saved owned material actually carries a known source texel there.  Writes
inspection/source-audit.png (source | saved materials | classes) and
inspection/source-audit.json.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
ROOT = HERE.parents[2]
ASSETS = ROOT / 'level-editor/work/needs-review/lincoln-refinement/round-1/assets'


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def mask_accept(ws, nodes, size):
    manifest = json.loads((ws / 'source-masks.json').read_text())
    inv_path = (ws / manifest['mask_inventory']).resolve()
    inv = {m['index']: m for m in json.loads(inv_path.read_text())['masks']}
    from PIL import Image
    W, H = size
    cache = {}

    def bitmap(i):
        if i not in cache:
            m = inv[i]
            full = np.zeros((H, W), bool)
            if m.get('png'):
                bm = np.asarray(Image.open(inv_path.parent / m['png']).convert('L')) > 0
                x0, y0 = m['box_top_left']
                h, w = bm.shape
                xs0, ys0 = max(0, x0), max(0, y0)
                xs1, ys1 = min(W, x0 + w), min(H, y0 + h)
                full[ys0:ys1, xs0:xs1] = bm[ys0 - y0:ys1 - y0, xs0 - x0:xs1 - x0]
            cache[i] = full
        return cache[i]
    per_node = {}
    for r in manifest['projections']['exterior']['assignments']:
        if r['source_node'] in nodes:
            acc = np.zeros((H, W), bool)
            for i in r['mask_indices']:
                acc |= bitmap(i)
            for i in r.get('exclude_mask_indices', []):
                acc &= ~bitmap(i)
            per_node[r['source_node']] = acc
    return per_node, manifest


def main():
    argv = sys.argv[sys.argv.index('--') + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument('--asset', required=True)
    ap.add_argument('--assets-dir', default=str(ASSETS))
    args = ap.parse_args(argv)
    ws = Path(args.assets_dir) / args.asset
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(ws / 'model.blend'))
    views = json.loads((ws / 'modified/views.json').read_text())
    crop = views['context_crop']
    left, top, right, bottom = crop['left'], crop['top'], crop['right'], crop['bottom']
    from PIL import Image
    source = Image.open(ws / 'reference/source.png').convert('RGB')
    size = source.size
    src = np.asarray(source)
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    toward = Vector((0, -cosine, sine))
    down = Vector((0, -sine, -cosine))
    verts, tris, recs = [], [], []
    owned_nodes = set()
    for obj in bpy.data.collections['lincoln Working'].all_objects:
        if obj.type != 'MESH' or obj.hide_render:
            continue
        pts = [obj.matrix_world @ v.co for v in obj.data.vertices]
        if not pts:
            continue
        px = [p.x for p in pts]
        py = [p.dot(down) for p in pts]
        if min(px) > right or max(px) < left or min(py) > bottom or max(py) < top:
            continue
        own = obj.get('asset_group') == args.asset
        if own:
            owned_nodes.add(obj.get('source_node'))
        off = len(verts)
        verts.extend(pts)
        obj.data.calc_loop_triangles()
        for t in obj.data.loop_triangles:
            tris.append(tuple(off + i for i in t.vertices))
            recs.append((obj, tuple(t.loops), t.material_index, own))
    tree = BVHTree.FromPolygons(verts, tris, all_triangles=True)
    accept, manifest = mask_accept(ws, owned_nodes, size)
    W, H = right - left, bottom - top
    mat_rgb = np.zeros((H, W, 3), np.uint8)
    cls = np.zeros((H, W), np.uint8)  # 0 none,1 foreign,2 owned-accepted-known,3 owned-accepted-unknown,4 owned-rejected
    img_cache = {}

    def sample(obj, loops, slot, hit, idx):
        mat = obj.data.materials[slot] if slot < len(obj.data.materials) else None
        if mat is None or not mat.use_nodes:
            return None
        texs = [n for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
        uvn = [n for n in mat.node_tree.nodes if n.type == 'UVMAP']
        if not texs:
            return None
        img = texs[-1].image
        uvname = uvn[-1].uv_map if uvn else obj.data.uv_layers.active.name
        if img.name not in img_cache:
            img_cache[img.name] = np.array(img.pixels[:], np.float32).reshape(img.size[1], img.size[0], 4)
        data = img_cache[img.name]
        uv = obj.data.uv_layers[uvname]
        a, b, c = (verts[i] for i in tris[idx])
        u, v, q = b - a, c - a, hit - a
        den = u.dot(u) * v.dot(v) - u.dot(v) ** 2
        bu = (q.dot(u) * v.dot(v) - q.dot(v) * u.dot(v)) / den
        bv = (q.dot(v) * u.dot(u) - q.dot(u) * u.dot(v)) / den
        st = uv.data[loops[0]].uv * (1 - bu - bv) + uv.data[loops[1]].uv * bu + uv.data[loops[2]].uv * bv
        x = int(min(max(st.x * img.size[0], 0), img.size[0] - 1))
        y = int(min(max(st.y * img.size[1], 0), img.size[1] - 1))
        return data[y, x]

    for sy in range(top, bottom):
        for sx in range(left, right):
            origin = Vector((sx + .5, 0, 0)) + down * (sy + .5) + toward * 100000
            hit, normal, idx, _ = tree.ray_cast(origin, -toward)
            X, Y = sx - left, sy - top
            if hit is None:
                continue
            obj, loops, slot, own = recs[idx]
            if not own:
                cls[Y, X] = 1
                mat_rgb[Y, X] = (40, 40, 60)
                continue
            texel = sample(obj, loops, slot, hit, idx)
            known = texel is not None and texel[3] > 0.5 and not (
                abs(texel[0] - texel[1]) < 1e-3 and abs(texel[1] - texel[2]) < 1e-3)
            if texel is not None:
                mat_rgb[Y, X] = np.clip(texel[:3] * 255, 0, 255).astype(np.uint8)
            ok = accept[obj.get('source_node')][sy, sx]
            cls[Y, X] = 4 if not ok else (2 if known else 3)
    colours = np.array([(0, 0, 0), (50, 50, 80), (40, 170, 60), (230, 200, 40), (220, 40, 40)], np.uint8)
    panel = colours[cls]
    crop_src = src[top:bottom, left:right]
    blend = (crop_src * 0.45 + panel * 0.55).astype(np.uint8)
    sheet = np.full((H, W * 3 + 20, 3), 255, np.uint8)
    sheet[:, :W] = crop_src
    sheet[:, W + 10:2 * W + 10] = mat_rgb
    sheet[:, 2 * W + 20:] = blend
    out = ws / 'inspection/source-audit.png'
    Image.fromarray(sheet).save(out)
    domain = int((cls >= 2).sum())
    counts = {'domain_owned_first_hit': domain, 'accepted_known': int((cls == 2).sum()),
              'accepted_without_known_texel': int((cls == 3).sum()),
              'mask_rejected': int((cls == 4).sum()), 'foreign_first_hit': int((cls == 1).sum()),
              'no_hit': int((cls == 0).sum())}
    rec = {'version': 1, 'asset_id': args.asset, 'crop': crop, 'counts': counts,
           'model_sha256': sha(ws / 'model.blend'), 'modified_views_sha256': sha(ws / 'modified/views.json'),
           'image': str(out), 'image_sha256': sha(out),
           'legend': 'panel 3 over source: green accepted with known saved texel, yellow accepted but saved texel '
                     'neutral/unknown, red owned first hit rejected by the working native-mask rule, blue foreign '
                     'first hit, black no geometry'}
    (ws / 'inspection/source-audit.json').write_text(json.dumps(rec, indent=1) + '\n')
    print(json.dumps(counts))


if __name__ == '__main__':
    main()
