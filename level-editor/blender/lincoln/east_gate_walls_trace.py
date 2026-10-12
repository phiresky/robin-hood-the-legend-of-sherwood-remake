"""Source-camera evidence images for the ``east_gate_walls`` lane (plain Python).

Rasterizes native-unit meshes with the original 35-degree orthographic camera
(pixel = (x, y - z)) and writes a three-panel comparison:
unmarked source crop | flat-shaded mesh | source with visible mesh edges and
numbered trace points.  Used for projection-alignment review; the frozen
eight-view packet remains the review contract.

usage:
  python3 east_gate_walls_trace.py <actual-native.json> <x0,y0,x1,y1> <scale> <out.png>
      [--nodes 78,94] [--context <native_all.json>] [--points <trace.json>]
"""
import argparse
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / 'level-editor/work/needs-review/lincoln-refinement/source-states/covered.png'
S = math.sin(math.radians(35))
C = math.cos(math.radians(35))
SUN = np.array([0.5712401270866394, -0.3459553122520447, 0.7443115711212158])


def world(p):
    return np.array([p[0], -p[1] / S, p[2] / C])


def rasterize(meshes, box, scale):
    """Return (face-id buffer, shade buffer); meshes: list of (verts, faces)."""
    x0, y0, x1, y1 = box
    w, h = (x1 - x0) * scale, (y1 - y0) * scale
    depth = np.full((h, w), -np.inf)
    fid = np.full((h, w), -1, np.int64)
    shade = np.zeros((h, w))
    base_id = 0
    for verts, faces in meshes:
        planes = {}
        V = np.array(verts, float)
        for f in faces:
            P = V[f]
            W = np.array([world(p) for p in P])
            n = np.cross(W[1] - W[0], W[2] - W[0])
            for k in range(2, len(f)):
                n2 = np.cross(W[k - 1] - W[0], W[k] - W[0])
                if np.linalg.norm(n2) > np.linalg.norm(n):
                    n = n2
            if np.linalg.norm(n) == 0:
                continue
            n = n / np.linalg.norm(n)
            plane = (tuple(np.round(n, 3)), round(float(n @ W[0]), 1))
            ident = planes.setdefault(plane, len(planes)) + base_id
            light = 0.25 + 0.75 * max(0.0, float(n @ SUN))
            sx = (P[:, 0] - x0) * scale
            sy = (P[:, 1] - P[:, 2] - y0) * scale
            dz = P[:, 1] * C / S + P[:, 2] * S / C
            for k in range(1, len(f) - 1):
                tri = [0, k, k + 1]
                xs, ys, ds = sx[tri], sy[tri], dz[tri]
                minx, maxx = int(max(0, math.floor(xs.min()))), int(min(w - 1, math.ceil(xs.max())))
                miny, maxy = int(max(0, math.floor(ys.min()))), int(min(h - 1, math.ceil(ys.max())))
                if minx > maxx or miny > maxy:
                    continue
                gx, gy = np.meshgrid(np.arange(minx, maxx + 1) + 0.5, np.arange(miny, maxy + 1) + 0.5)
                d = (ys[1] - ys[2]) * (xs[0] - xs[2]) + (xs[2] - xs[1]) * (ys[0] - ys[2])
                if abs(d) < 1e-12:
                    continue
                a = ((ys[1] - ys[2]) * (gx - xs[2]) + (xs[2] - xs[1]) * (gy - ys[2])) / d
                b = ((ys[2] - ys[0]) * (gx - xs[2]) + (xs[0] - xs[2]) * (gy - ys[2])) / d
                c = 1 - a - b
                inside = (a >= -1e-6) & (b >= -1e-6) & (c >= -1e-6)
                zz = a * ds[0] + b * ds[1] + c * ds[2]
                sub = depth[miny:maxy + 1, minx:maxx + 1]
                win = inside & (zz > sub)
                sub[win] = zz[win]
                fid[miny:maxy + 1, minx:maxx + 1][win] = ident
                shade[miny:maxy + 1, minx:maxx + 1][win] = light
        base_id += 100000
    return fid, shade


def panels(meshes, box, scale, points=(), context=()):
    x0, y0, x1, y1 = box
    src = Image.open(SOURCE).convert('RGB').crop(box)
    src = src.resize(((x1 - x0) * scale, (y1 - y0) * scale), Image.NEAREST)
    fid, shade = rasterize(list(context) + list(meshes), box, scale)
    own_ids = rasterize(list(meshes), box, scale)[0]
    ctx_count = 100000 * len(context)
    owned_visible = (fid >= ctx_count)
    gray = (shade * 230).astype(np.uint8)
    solid = np.stack([gray] * 3, -1)
    solid[fid < 0] = (20, 20, 30)
    solid[(fid >= 0) & ~owned_visible] = (solid[(fid >= 0) & ~owned_visible] * 0.45).astype(np.uint8)
    edge = np.zeros(fid.shape, bool)
    edge[:, 1:] |= fid[:, 1:] != fid[:, :-1]
    edge[1:, :] |= fid[1:, :] != fid[:-1, :]
    edge &= (owned_visible | np.roll(owned_visible, 1, 0) | np.roll(owned_visible, 1, 1))
    edge &= (fid >= 0) | np.roll(fid >= 0, 1, 0) | np.roll(fid >= 0, 1, 1)
    over = np.array(src).copy()
    over[edge] = (255, 40, 40)
    over_img = Image.fromarray(over)
    draw = ImageDraw.Draw(over_img)
    for i, (px, py) in enumerate(points):
        X, Y = (px - x0) * scale, (py - y0) * scale
        draw.ellipse([X - 3, Y - 3, X + 3, Y + 3], outline=(0, 255, 255))
        draw.text((X + 4, Y - 10), str(i + 1), fill=(0, 255, 255))
    W, H = src.size
    out = Image.new('RGB', (W * 3 + 20, H), (0, 0, 0))
    out.paste(src, (0, 0))
    out.paste(Image.fromarray(solid), (W + 10, 0))
    out.paste(over_img, (2 * W + 20, 0))
    del own_ids
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('actual')
    ap.add_argument('box')
    ap.add_argument('scale', type=int)
    ap.add_argument('out')
    ap.add_argument('--nodes')
    ap.add_argument('--context', help='native JSON of neighbouring meshes (occluders)')
    ap.add_argument('--context-nodes')
    ap.add_argument('--points', help='JSON list of [x, y] source pixels to number')
    a = ap.parse_args()
    data = json.loads(Path(a.actual).read_text())
    keep = set(a.nodes.split(',')) if a.nodes else set(data)
    meshes = [(v['verts'], v['faces']) for k, v in data.items() if k in keep]
    context = []
    if a.context:
        ctx = json.loads(Path(a.context).read_text())
        for k in (a.context_nodes.split(',') if a.context_nodes else []):
            for o in (ctx[k] if isinstance(ctx[k], list) else [ctx[k]]):
                context.append((o['verts'], o['faces']))
    points = json.loads(Path(a.points).read_text()) if a.points else []
    box = tuple(int(v) for v in a.box.split(','))
    panels(meshes, box, a.scale, points, context).save(a.out)
    print(a.out)


if __name__ == '__main__':
    main()
