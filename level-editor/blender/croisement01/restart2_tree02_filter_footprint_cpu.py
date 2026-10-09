"""Bound residual diagnostic pixels against the existing exact triangle packet.

This deliberately includes occluded triangles. It identifies probe blind spots;
it cannot authorize a fill or claim that every candidate contributes to a pixel.
"""
import hashlib
import json
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
P = R / 'tree02-rendered-gap-probe-v2/report.json'
B = R / 'approved-tree02-fill-v1/croisement01-tree-02/baked-v4-bounded-bark-gaps'
O = R / 'tree02-filter-footprint-cpu-v1'
assert not O.exists()
packet = json.loads(P.read_text())['packets']['Tree02 upper stems']
manifest = json.loads((B / 'coverage-views-384.json').read_text())
ownership = np.load(B / 'upper-stems-provenance.npz')['ownership']


def clip(poly, axis, bound, sign):
    result = []
    for a, b in zip(poly, np.roll(poly, -1, axis=0)):
        da, db = sign * (a[axis] - bound), sign * (b[axis] - bound)
        if da >= 0:
            result.append(a)
        if (da >= 0) != (db >= 0):
            result.append(a + (b - a) * da / (da - db))
    return np.asarray(result)


def rectangle(poly, lo, hi):
    for axis in range(2):
        for bound, sign in [(lo[axis], 1), (hi[axis], -1)]:
            poly = clip(poly, axis, bound, sign)
            if len(poly) < 3:
                return np.empty((0, poly.shape[-1] if poly.ndim == 2 else 2))
    return poly


rows = []
for vi, x, y in [(0, 148, 94), (0, 149, 94), (0, 149, 95), (1, 192, 155), (3, 218, 79)]:
    view = next(v for v in manifest['views'] if v['index'] == vi)
    inverse = np.linalg.inv(np.asarray(view['camera_matrix_world']))
    candidates = {}
    for tri in packet['triangles']:
        xyz = np.asarray([packet['vertices_world'][str(v)] for v in tri['vertices']])
        camera = np.c_[xyz, np.ones(3)] @ inverse.T
        screen = (camera[:, :2] / view['ortho_scale'] * [1, -1] + .5) * 384
        # Three-pixel square is a conservative diagnostic bound, not a claim
        # about the saved render's filter radius or visible first-hit surfaces.
        polygon = rectangle(np.c_[screen, np.asarray(tri['uv']) * packet['atlas_size']], [x - 1, y - 1], [x + 2, y + 2])
        if not len(polygon):
            continue
        uv = polygon[:, 2:]
        lo = np.maximum(0, np.floor(uv.min(0)).astype(int))
        hi = np.minimum(np.array(packet['atlas_size']) - 1, np.floor(uv.max(0)).astype(int))
        for ty in range(lo[1], hi[1] + 1):
            for tx in range(lo[0], hi[0] + 1):
                if ownership[ty, tx] != 0:
                    continue
                cut = rectangle(uv, [tx, ty], [tx + 1, ty + 1])
                if len(cut) >= 3:
                    area = abs(np.dot(cut[:, 0], np.roll(cut[:, 1], -1)) - np.dot(cut[:, 1], np.roll(cut[:, 0], -1))) / 2
                    if area > 1e-10:
                        candidates.setdefault(f'{tx},{ty}', []).append(tri['face'])
    rows.append(dict(view=vi, pixel=[x, y], unfilled_candidate_faces=candidates))
result = dict(status='CONSERVATIVE PARTIAL-MESH FOOTPRINT; VISIBILITY UNPROVEN',
              packet_sha256=hashlib.sha256(P.read_bytes()).hexdigest(),
              saved_model_sha256=hashlib.sha256((B / 'worker.blend').read_bytes()).hexdigest(),
              assumptions=['Square orthographic pixels; camera manifest is authoritative.',
                           'Only previously exported witness and adjacent faces are available.',
                           'Three-pixel square includes occluded geometry; no donor or fill authorization.'],
              pixels=rows)
O.mkdir()
(O / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps([dict(view=r['view'], pixel=r['pixel'], remaining_candidates=len(r['unfilled_candidate_faces'])) for r in rows]))
