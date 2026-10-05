"""Partition a tentative static shrub domain without claiming material ownership."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/croisement03-refinement'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    preflight = WORK / 'restart2/east-shrub61-preflight'
    output = WORK / 'restart2/east-shrub61-source-lobes-v1'
    output.mkdir(exist_ok=False)
    source_path = WORK / 'baseline/covered.png'
    domain_path = preflight / 'conservative-plant-domain.png'
    source = np.array(Image.open(source_path).convert('RGB'))
    domain = np.array(Image.open(domain_path).convert('L')) > 0
    assert source.shape[:2] == domain.shape and int(domain.sum()) == 4339
    # Manual cluster centers guide construction only, not original plant topology.
    seeds = np.array([(1053, 399), (1075, 382), (1107, 415), (1077, 441), (1036, 423)])
    ys, xs = np.nonzero(domain)
    labels = ((xs[:, None] - seeds[:, 0]) ** 2 + (ys[:, None] - seeds[:, 1]) ** 2).argmin(axis=1)
    union = np.zeros_like(domain)
    rows = []
    for index, seed in enumerate(seeds):
        owned = np.zeros_like(domain)
        owned[ys[labels == index], xs[labels == index]] = True
        assert not np.any(union & owned)
        union |= owned
        y, x = np.nonzero(owned)
        box = [int(x.min()), int(y.min()), int(x.max()) + 1, int(y.max()) + 1]
        x0, y0, x1, y1 = box
        rgba = np.zeros((y1 - y0, x1 - x0, 4), dtype=np.uint8)
        rgba[:, :, :3] = source[y0:y1, x0:x1]
        rgba[:, :, 3] = owned[y0:y1, x0:x1] * 255
        rgba[rgba[:, :, 3] == 0, :3] = 0
        native = output / f'lobe-{index:02}-candidate-native.png'
        Image.fromarray(rgba).save(native)
        rgba[:, :, :3] = 105
        rgba[rgba[:, :, 3] == 0, :3] = 0
        unknown = output / f'lobe-{index:02}-unknown.png'
        Image.fromarray(rgba).save(unknown)
        rows.append(dict(index=index, seed=seed.tolist(), bbox=box, pixels=int(owned.sum()),
                         candidate_native_sha256=sha(native), unknown_sha256=sha(unknown)))
    assert np.array_equal(union, domain)
    report = dict(status='Private source construction preflight; geometry and final ownership unapproved',
                  source_sha256=sha(source_path), domain_sha256=sha(domain_path),
                  candidate_pixels=int(domain.sum()), lobes=rows,
                  exclusions=['148 traced stone overlap pixels reserved before partitioning'],
                  limitations=[
                      'Five cluster centers and their partition are inferred construction aids, not a measured stem count.',
                      'Dark gaps and mixed material ownership still need saved-mesh source review.',
                      'Static shrub colors remain distinct from animated crown frames; overlap is not blanket subtraction.',
                      'No height, depth, ground position, obstacle or gameplay rule is inferred by this partition.',
                  ])
    (output / 'partition.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'output': str(output), 'pixels': int(domain.sum()), 'lobes': len(rows)}))


if __name__ == '__main__':
    main()
