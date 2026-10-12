"""Measure local native FX correspondence without inferring material ownership."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.ndimage import uniform_filter

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement01-refinement/restart2'
OUT = BASE / 'tree14-local-fx-correspondence-v1'
OUT.mkdir(exist_ok=False)
frames = json.loads((BASE / 'tree14-native-crown-animation-source-v1/source.json').read_text())['frames']
left, top, width, height = 640, 35, 350, 315
arrays = []
for frame in frames:
    path = ROOT / frame['path']
    assert hashlib.sha256(path.read_bytes()).hexdigest() == frame['sha256']
    raw = np.asarray(Image.open(path).convert('RGBA'))
    x, y, w, h = frame['bbox']
    aligned = np.zeros((height, width, 4), np.uint8)
    aligned[y-top:y-top+h, x-left:x-left+w] = raw
    arrays.append(aligned)
reference = arrays[0]
reference_active = reference[:, :, 3] > 0
reference_weight = uniform_filter(reference_active.astype(float), size=7, mode='constant')
records = []
for frame_index in (0, 3, 7, 11):
    target = arrays[frame_index]
    candidates = []
    for dy in range(-3, 4):
        for dx in range(-3, 4):
            shifted = np.zeros_like(target)
            y0, y1, x0, x1 = max(0, -dy), min(height, height-dy), max(0, -dx), min(width, width-dx)
            shifted[y0:y1, x0:x1] = target[y0+dy:y1+dy, x0+dx:x1+dx]
            valid = reference_active & (shifted[:, :, 3] > 0)
            error = np.abs(reference[:, :, :3].astype(float)-shifted[:, :, :3].astype(float)).mean(axis=2)
            weight = uniform_filter(valid.astype(float), size=7, mode='constant')
            error_sum = uniform_filter(error * valid, size=7, mode='constant')
            score = error_sum / np.maximum(weight, 1e-12) + 15 * (1-weight/np.maximum(reference_weight, 1e-12))
            score[(weight < reference_weight * .5) | (reference_weight < 5/49)] = 1e6
            candidates.append((score, dx, dy))
    scores = np.stack([row[0] for row in candidates])
    order = np.argsort(scores, axis=0)
    yy, xx = np.indices((height, width))
    best = order[0]
    first, second = scores[best, yy, xx], scores[order[1], yy, xx]
    offsets = np.array([[row[1], row[2]] for row in candidates], dtype=np.int8)
    flow = offsets[best]
    valid = reference_active & (first < 1e6)
    confident = valid & (second-first >= 3)
    np.savez_compressed(OUT / f'frame-{frame_index:02d}.npz', displacement=flow,
                        score=first.astype(np.float32), margin=(second-first).astype(np.float32),
                        measured=valid, locally_distinct=confident)
    color = np.zeros((height, width, 3), np.uint8)
    color[:, :, 0] = np.where(valid, (flow[:, :, 0].astype(int)+3)*36, 0)
    color[:, :, 1] = np.where(valid, (flow[:, :, 1].astype(int)+3)*36, 0)
    color[:, :, 2] = np.where(confident, 220, 50)
    Image.fromarray(color).save(OUT / f'frame-{frame_index:02d}-displacements.png')
    histogram = []
    for dx, dy in offsets:
        count = int(np.count_nonzero(confident & (flow[:, :, 0] == dx) & (flow[:, :, 1] == dy)))
        if count:
            histogram.append(dict(dx=int(dx), dy=int(dy), samples=count))
    records.append(dict(frame=frame_index, measured=int(valid.sum()), locally_distinct=int(confident.sum()),
                        mean_score=float(first[valid].mean()), displacements=histogram))
(OUT / 'report.json').write_text(json.dumps(dict(
    status='MEASURED_LOCAL_CORRESPONDENCE_NOT_MATERIAL_CLASSIFICATION',
    source_window=[left, top, left+width, top+height], patch_size=7,
    comparison_frames=records,
    limitations=['RGB patch agreement measures correspondence only; it cannot distinguish bark from foliage.',
                 'Native dithered alpha is preserved; absent pixels are never filled for this measurement.',
                 'Occlusion, mixed-material windows and repeating detail can make local matches ambiguous.',
                 'No geometry, source ownership, animation resource or texture-fill domain is modified.']), indent=2)+'\n')
print([(r['frame'], r['locally_distinct'], r['mean_score']) for r in records])
