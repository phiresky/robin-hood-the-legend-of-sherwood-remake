"""Create a continuous branch-depth hypothesis from the native wood silhouette."""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.ndimage import distance_transform_edt, gaussian_filter

from catalog import OUT


def build(output, depth_smoothing=4.):
    record = json.loads((OUT / 'baseline/Croisement02.rhp.json').read_text())['masks'][44]
    native = np.asarray(Image.open(OUT / 'baseline/masks/000044.png').convert('L')) > 0
    factor, pad, extension = 2, 4, 30
    h, w = native.shape
    mask = np.zeros((h + extension + pad * 2, w + pad * 2), bool)
    mask[pad:pad + h, pad:pad + w] = native
    # Continue the observed bottom stem beyond the map, retaining the previous
    # candidate's thirty-pixel extension and gentle fifteen-percent widening.
    ends = np.flatnonzero(native[-1])
    center, radius = (ends[0] + ends[-1] + 1) / 2, len(ends) / 2
    for row in range(extension):
        r = radius * (1 + .15 * (row + 1) / extension)
        cols = np.arange(w) + .5
        mask[pad + h + row, pad:pad + w] = abs(cols - center) <= r
    mask = np.repeat(np.repeat(mask, factor, axis=0), factor, axis=1)
    distance = distance_transform_edt(mask) / factor
    depth = gaussian_filter(np.sqrt(distance * float(distance.max())), sigma=depth_smoothing)
    sin, cos = np.sin(np.deg2rad(35)), np.cos(np.deg2rad(35))
    vertices, faces, indices = [], [], {}

    def vertex(x, y, side):
        key = x, y, side
        if key not in indices:
            local = depth[max(0, y - 1):y + 1, max(0, x - 1):x + 1]
            d = float(local.mean())
            if not mask[max(0, y - 1):y + 1, max(0, x - 1):x + 1].all():
                d = .02
            px = record['box_top_left'][0] + x / factor - pad
            py = record['box_top_left'][1] + y / factor - pad
            q = side * d / cos
            indices[key] = len(vertices)
            vertices.append([px, -1180 / sin - cos * q, (1180 - py) / cos + sin * q])
        return indices[key]

    for y, x in zip(*np.nonzero(mask)):
        front = [vertex(x, y, 1), vertex(x + 1, y, 1), vertex(x + 1, y + 1, 1), vertex(x, y + 1, 1)]
        back = [vertex(x, y, -1), vertex(x + 1, y, -1), vertex(x + 1, y + 1, -1), vertex(x, y + 1, -1)]
        faces.extend([front, list(reversed(back))])
        for a, b, dy, dx in [(0, 1, -1, 0), (1, 2, 0, 1), (2, 3, 1, 0), (3, 0, 0, -1)]:
            if not mask[y + dy, x + dx]:
                faces.append([front[b], front[a], back[a], back[b]])
    output.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(output, vertices=np.asarray(vertices), faces=np.asarray(faces))
    print(output, len(vertices), len(faces))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--depth-smoothing', type=float, default=4.)
    args = parser.parse_args()
    build(args.output, args.depth_smoothing)
