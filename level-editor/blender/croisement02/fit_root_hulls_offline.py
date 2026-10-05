"""Local convex branch contour additions, preserving every prior tube volume."""
import argparse
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.spatial import ConvexHull
from fit_root_radii_offline import OUT, SIN, COS, inside


def tube(a, b):
    def world(p):
        x, y, z, radius = p
        return np.array((x, -(y + z * COS) / SIN, z))
    aa, bb = world(a), world(b); axis = bb - aa; axis /= np.linalg.norm(axis)
    helper = (0., 0., 1.) if abs(axis[2]) < .9 else (1., 0., 0.)
    u = np.cross(axis, helper); u /= np.linalg.norm(u); v = np.cross(axis, u)
    return np.array([center + p[3] * (u * math.cos(k * math.tau / 12) + v * math.sin(k * math.tau / 12)) for center, p in [(aa, a), (bb, b)] for k in range(12)])


def screen(p):
    return np.column_stack((p[:, 0], -p[:, 1] * SIN - p[:, 2] * COS))


def nearest(points, target):
    projected = screen(points); indices = ConvexHull(projected).vertices
    a = projected[indices]; b = np.roll(a, -1, axis=0); d = b - a
    t = np.clip(np.sum((target - a) * d, axis=1) / np.sum(d * d, axis=1), 0, 1)
    q = a + t[:, None] * d; index = np.argmin(np.linalg.norm(q - target, axis=1))
    world = points[indices[index]] * (1 - t[index]) + points[indices[(index + 1) % len(indices)]] * t[index]
    return float(np.linalg.norm(q[index] - target)), q[index], world


def main(kind):
    folder = 'logging-branches-v5' if kind == 'logging' else 'southwest-branches-v1'
    worker = OUT / 'restart2-vegetation' / folder; proposal = json.loads((worker / 'proposal.json').read_text())
    targets = np.array([r['pixel'] for r in proposal['hits'] if r['object'] is None], float) + .5
    segments = [(a, b) for path in proposal['traced_source_paths'] for a, b in zip(path, path[1:])]
    parts = [tube(a, b) for a, b in segments]; originals = [p.copy() for p in parts]
    source = np.asarray(Image.open(worker / 'front-source.png')); additions = []
    remaining = list(range(len(targets))); rejected = []
    while remaining:
        target_index = remaining.pop(0); target = targets[target_index]
        if any(inside(screen(p), target[None, :])[0] for p in parts):
            continue
        candidates = []
        for index, points in enumerate(parts):
            distance, q, world = nearest(points, target)
            if distance > 4 or distance < 1e-8:
                continue
            offset = (target - q) * (1 + .3 / distance)
            added = world + np.array((offset[0], -SIN * offset[1], -COS * offset[1]))
            if added[2] < .1:
                added += (.1 - added[2]) / SIN * np.array((0., -COS, SIN))
            displacement = float(np.linalg.norm(added - world))
            if displacement > 8:
                continue
            extended = np.vstack((points, added)); projected = screen(extended)
            lo = np.floor(projected.min(axis=0) - 1).astype(int); hi = np.ceil(projected.max(axis=0) + 1).astype(int)
            yy, xx = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]; grid = np.column_stack((xx.ravel() + .5, yy.ravel() + .5))
            novel = inside(projected, grid) & ~inside(screen(points), grid)
            foreign = int((novel & (source[yy, xx, 3].ravel() == 0)).sum())
            candidates.append((foreign * 4 + int(novel.sum()) + displacement, index, extended, foreign, displacement))
        if not candidates:
            rejected.append(target_index); continue
        _, index, extended, foreign, displacement = min(candidates, key=lambda r: r[0])
        parts[index] = extended
        additions.append(dict(segment=index, target=(target - .5).astype(int).tolist(), added_point=extended[-1].tolist(), added_foreign=foreign, local_displacement=displacement))
    covered = np.zeros(len(targets), bool)
    for points in parts:
        covered |= inside(screen(points), targets)
    dest = OUT / 'restart2-vegetation' / (kind + '-convex-contour-research'); dest.mkdir(exist_ok=True)
    report = dict(status='Private analytic convex-volume proposal; independent BVH/material/neighbor review required', original_model_sha256=proposal['model_sha256'], targets=len(targets), covered=int(covered.sum()), residual_pixels=(targets[~covered] - .5).astype(int).tolist(), additions=additions, parts=[dict(segment=i, original_vertices=originals[i].tolist(), vertices=p.tolist(), triangles=ConvexHull(p).simplices.tolist()) for i, p in enumerate(parts)], limitations=['Each closed convex tube volume is retained; local contour points only added.', 'Source ownership unchanged; inferred geometry point placement is not observed depth.', 'Native material assignment, contacts and source extras require independent Blender review.'])
    (dest / 'proposal.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(kind=kind, targets=len(targets), covered=int(covered.sum()), additions=len(additions), added_foreign_sum=sum(r['added_foreign'] for r in additions))))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('kind', choices=['logging', 'southwest']); main(parser.parse_args().kind)
