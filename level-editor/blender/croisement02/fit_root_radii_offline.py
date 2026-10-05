"""Bounded segment-radius proposals using exact convex projected tube hulls.

This research does not write Blender models or change source authority.
"""
import argparse
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.spatial import ConvexHull

OUT = Path(__file__).resolve().parents[3] / 'level-editor/work/croisement02-refinement'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))


def projected_tube(a, b, addition):
    def world(p):
        x, y, z, radius = p
        return np.array((x, -(y + z * COS) / SIN, z))
    aa, bb = world(a), world(b); axis = bb - aa; axis /= np.linalg.norm(axis)
    helper = (0., 0., 1.) if abs(axis[2]) < .9 else (1., 0., 0.)
    u = np.cross(axis, helper); u /= np.linalg.norm(u); v = np.cross(axis, u)
    points = np.array([center + (p[3] + addition) * (u * math.cos(k * math.tau / 12) + v * math.sin(k * math.tau / 12)) for center, p in [(aa, a), (bb, b)] for k in range(12)])
    return np.column_stack((points[:, 0], -points[:, 1] * SIN - points[:, 2] * COS))


def inside(hull, points):
    eq = ConvexHull(hull).equations
    return np.all(points @ eq[:, :2].T + eq[:, 2] <= 1e-7, axis=1)


def main(kind):
    folder = 'logging-branches-v5' if kind == 'logging' else 'southwest-branches-v1'
    worker = OUT / 'restart2-vegetation' / folder
    proposal = json.loads((worker / 'proposal.json').read_text())
    targets = np.array([r['pixel'] for r in proposal['hits'] if r['object'] is None], float) + .5
    segments = [(a, b) for path in proposal['traced_source_paths'] for a, b in zip(path, path[1:])]
    source = np.asarray(Image.open(worker / 'front-source.png'))
    lo = np.floor(targets.min(axis=0) - 12).astype(int); hi = np.ceil(targets.max(axis=0) + 12).astype(int)
    yy, xx = np.mgrid[lo[1]:hi[1], lo[0]:hi[0]]; grid = np.column_stack((xx.ravel() + .5, yy.ravel() + .5))
    owned = source[yy, xx, 3].ravel() > 0
    baseline = np.zeros(len(grid), bool)
    for a, b in segments:
        baseline |= inside(projected_tube(a, b, 0), grid)
    choices = []
    for index, (a, b) in enumerate(segments):
        for addition in [.5, 1., 1.5, 2., 2.5, 3.]:
            hull = projected_tube(a, b, addition); covered = inside(hull, targets)
            if covered.any():
                pixels = inside(hull, grid)
                choices.append(dict(segment=index, addition=addition, covered=covered, extras=int((pixels & ~owned & ~baseline).sum())))
    remaining = np.ones(len(targets), bool); chosen = {}
    while remaining.any():
        candidates = [r for r in choices if r['addition'] > chosen.get(r['segment'], 0) and (r['covered'] & remaining).any()]
        if not candidates:
            break
        best = max(candidates, key=lambda r: int((r['covered'] & remaining).sum()) / (1 + r['extras'] + r['addition']))
        chosen[best['segment']] = best['addition']; remaining &= ~best['covered']
    after = np.zeros(len(grid), bool)
    for index, (a, b) in enumerate(segments):
        after |= inside(projected_tube(a, b, chosen.get(index, 0)), grid)
    dest = OUT / 'restart2-vegetation' / (kind + '-radius-research'); dest.mkdir(exist_ok=True)
    report = dict(status='Private analytic proposal; no geometry mutation', kind=kind, original_model_sha256=proposal['model_sha256'], targets=len(targets), covered=int((~remaining).sum()), residual_pixels=(targets[remaining] - .5).astype(int).tolist(), max_radius_addition=3., segments=[dict(index=i, a=a, b=b, radius_addition=chosen.get(i, 0)) for i, (a, b) in enumerate(segments)], new_foreign_projection_pixels=int((after & ~owned & ~baseline).sum()), old_projected_hits_lost=int((baseline & ~after).sum()), limitations=['Independent Blender center-ray and material review still required.', 'Radial growth preserves each closed convex segment; intersections and ground contacts still require review.', 'Unreachable contours require explicit traced limb construction, not larger global radii.'])
    (dest / 'proposal.json').write_text(json.dumps(report, indent=2) + '\n')
    overlay = source[lo[1]:hi[1], lo[0]:hi[0]].copy(); overlay[:, :, 3] = 255
    overlay.reshape(-1, 4)[after & ~owned & ~baseline, :3] = (0, 170, 255)
    for x, y in targets[remaining] - .5:
        overlay[int(y) - lo[1], int(x) - lo[0], :3] = (255, 0, 120)
    Image.fromarray(overlay).resize((overlay.shape[1] * 4, overlay.shape[0] * 4), Image.Resampling.NEAREST).save(dest / 'source-residual.png')
    print(json.dumps({k: report[k] for k in ['kind', 'targets', 'covered', 'new_foreign_projection_pixels', 'old_projected_hits_lost']}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('kind', choices=['logging', 'southwest']); main(parser.parse_args().kind)
