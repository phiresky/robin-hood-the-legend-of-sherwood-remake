"""Separate the eastern background tree from its foreground neighbour's crown."""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

from catalog import OUT, tree_workspace

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from evidence_io import sha, write_json


def main():
    worker = tree_workspace(40)
    decisions = json.loads((OUT / 'user-feedback.json').read_text())['records']
    own = [r for r in decisions if r['asset_id'] == worker.name]
    if own and own[-1]['decision'] == 'approved':
        raise ValueError('Approved geometry is frozen')
    original = worker / 'inspection/irregular-crown-edge/partition.json'
    packet = json.loads(original.read_text())
    path = original.parent / 'complete-source.png'
    rgba = np.asarray(Image.open(path).convert('RGBA')).copy()
    x, y, width, height = packet['native_bbox']
    neighbour = tree_workspace(39)
    report = json.loads((neighbour / 'inspection/refinement.json').read_text())
    other_path = Path(report['source_packet'])
    other = json.loads(other_path.read_text())
    ox, oy, ow, oh = other['native_bbox']
    other_alpha = np.asarray(Image.open(other_path.parent / 'complete-source.png').convert('RGBA'))[:, :, 3] > 127
    full = np.zeros((1152, 1792), bool)
    full[oy:oy+oh, ox:ox+ow] = other_alpha
    alpha = rgba[:, :, 3] > 127
    if np.any(alpha & ~full[y:y+height, x:x+width]):
        raise ValueError('Neighbour does not cover the former shared domain')
    yy, xx = np.indices(alpha.shape)
    # The native wood mask identifies this rear trunk at x=1707..1781.
    # Its individual crown edge is hidden; this overlap partition is inferred.
    dx = xx + x - 1792
    dy = yy + y - 365
    angle = np.arctan2(dy / 106, dx / 108)
    edge = 1 + .055 * np.sin(angle * 7) + .035 * np.cos(angle * 11)
    domain = (dx / 108) ** 2 + (dy / 106) ** 2 < edge ** 2
    rgba[:, :, 3][~domain] = 0
    observed = rgba[:, :, 3] > 127
    ys, xs = np.where(observed)
    if not len(xs):
        raise ValueError('Empty revised crown')
    destination = worker / 'inspection/eastern-background-crown'
    destination.mkdir(exist_ok=True)
    Image.fromarray(rgba).save(destination / 'complete-source.png')
    packet.update(bbox=[int(x+xs.min()), int(y+ys.min()), int(np.ptp(xs)+1), int(np.ptp(ys)+1)],
        lobes=[dict(image=str(destination / 'complete-source.png'))], source_pixels=int(observed.sum()),
        coverage_provenance='Inferred individual crown around the rear eastern trunk. The foreground tree 39 retains the complete former shared domain.')
    write_json(destination / 'partition.json', packet)
    write_json(destination / 'ownership-review.json', dict(previous_packet_sha256=sha(original),
        previous_image_sha256=sha(path), neighbour_packet_sha256=sha(other_path),
        neighbour_image_sha256=sha(other_path.parent / 'complete-source.png'),
        removed_pixels=int((alpha & ~observed).sum()), removed_pixels_without_neighbour=0,
        evidence=str(OUT / 'tree40-wood-context.png'), evidence_sha256=sha(OUT / 'tree40-wood-context.png'),
        limitation='Source-domain coverage only; final joint geometry coverage still requires integrated review.'))
    own_report = worker / 'inspection/refinement.json'
    value = json.loads(own_report.read_text())
    value['source_packet'] = str(destination / 'partition.json')
    write_json(own_report, value)
    write_json(worker / 'inspection/bark-donor-selection.json', dict(native_mask=40,
        source_box=[1745, 505, 1752, 512], source_sha256=sha(worker / 'reference/source.png'),
        reviewer='Codex', evidence=str(OUT / 'tree40-bark-candidates.png'),
        evidence_sha256=sha(OUT / 'tree40-bark-candidates.png'),
        notes='Inspected exposed dark vertical bark between canopy leaves. Lower samples contain green foreground and are excluded. Hidden bark grain is inferred.'))


if __name__ == '__main__':
    main()
