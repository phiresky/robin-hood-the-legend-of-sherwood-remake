"""Source-coverage audit for one ``east_gate_walls`` workspace (plain Python).

The audit domain is derived independently of the acceptance masks: the saved
owned meshes (``inspection/actual-native.json``) are rasterized from the
original source camera together with every other scene mesh as occluder
(baseline native geometry of the neighbouring assets, as present in the
workspace).  Pixels where an owned mesh is the first hit form the
geometric domain.  It is then compared with the reviewed native masks of the
workspace's ``source-masks.json``:

* accepted  - owned first hit and inside the reviewed mask
* rejected  - owned first hit but outside the mask (or inside an exclusion)
* uncovered - inside the mask, but the first hit is not an owned mesh

The evidence image shows the unmarked source beside the classified overlay
(green accepted, red rejected, blue uncovered).  The JSON result is written to
``inspection/coverage-classes.json``; the worker writes the hash-bound
``source-coverage-audit.json`` after inspecting it.

usage: python3 east_gate_walls_audit.py <workspace> <native_all.json>
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from east_gate_walls_trace import rasterize, SOURCE  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
INV = ROOT / 'level-editor/work/needs-review/lincoln-refinement/mask-review/inventory-v1'


def mask_image(indices):
    manifest = {e['index']: e for e in json.loads((INV / 'manifest.json').read_text())['masks']}
    full = np.zeros((2176, 2944), bool)
    for i in indices:
        e = manifest[i]
        m = np.array(Image.open(INV / e['png']).convert('L')) > 0
        x0, y0 = e['box_top_left']
        h, w = m.shape
        full[y0:y0 + h, x0:x0 + w] |= m
    return full


def main():
    workspace = Path(sys.argv[1]).resolve()
    scene = json.loads(Path(sys.argv[2]).read_text())
    actual = json.loads((workspace / 'inspection/actual-native.json').read_text())
    masks = json.loads((workspace / 'source-masks.json').read_text())
    owned_nodes = {f'building-{int(k.split(":")[0]):03d}' for k in actual}
    include, exclude = set(), set()
    rows = masks['projections']['exterior']['assignments']
    for row in rows:
        if row['source_node'] in owned_nodes:
            include.update(row['mask_indices'])
            exclude.update(row.get('exclude_mask_indices', []))
    include.discard(428)
    accept = mask_image(sorted(include)) & ~mask_image(sorted(exclude)) if include else np.zeros((2176, 2944), bool)
    owned = [(v['verts'], v['faces']) for v in actual.values()]
    xs = [p[0] for v, _ in owned for p in v]
    ys = [p[1] - p[2] for v, _ in owned for p in v]
    ys_m, xs_m = np.nonzero(accept)
    x0 = int(max(0, min(min(xs), xs_m.min() if len(xs_m) else 1e9) - 8))
    x1 = int(min(2944, max(max(xs), xs_m.max() if len(xs_m) else -1e9) + 8))
    y0 = int(max(0, min(min(ys), ys_m.min() if len(ys_m) else 1e9) - 8))
    y1 = int(min(2176, max(max(ys), ys_m.max() if len(ys_m) else -1e9) + 8))
    box = (x0, y0, x1, y1)
    context = []
    asset_id = json.loads((workspace / 'workspace.json').read_text())['asset_id']
    for node, objs in scene.items():
        for o in objs:
            if o.get('asset_group') == asset_id or (
                    o.get('asset_group') is None and f'building-{int(node):03d}' in owned_nodes):
                continue  # owned (component-aware); foreign components stay occluders
            if o.get('hide_render'):
                continue
            v = o['verts']
            px = [p[0] for p in v]
            py = [p[1] - p[2] for p in v]
            if max(px) < x0 or min(px) > x1 or max(py) < y0 or min(py) > y1:
                continue
            context.append((v, o['faces']))
    fid, _ = rasterize(context + owned, box, 1)
    first_owned = fid >= 100000 * len(context)
    acc = accept[y0:y1, x0:x1]
    accepted = first_owned & acc
    rejected = first_owned & ~acc
    uncovered = acc & ~first_owned
    foreign = uncovered & (fid >= 0)
    empty = uncovered & (fid < 0)
    src = np.array(Image.open(SOURCE).convert('RGB').crop(box))
    over = src.copy()
    over[accepted] = (over[accepted] * 0.4 + np.array([0, 255, 0]) * 0.6).astype(np.uint8)
    over[rejected] = (over[rejected] * 0.4 + np.array([255, 0, 0]) * 0.6).astype(np.uint8)
    over[uncovered] = (over[uncovered] * 0.4 + np.array([40, 80, 255]) * 0.6).astype(np.uint8)
    h, w = accepted.shape
    sheet = Image.new('RGB', (w * 2 + 10, h), (0, 0, 0))
    sheet.paste(Image.fromarray(src), (0, 0))
    sheet.paste(Image.fromarray(over), (w + 10, 0))
    out = workspace / 'inspection/source-coverage.png'
    sheet.save(out)
    result = {'box': box, 'mask_indices': sorted(include), 'exclude_mask_indices': sorted(exclude),
              'owned_first_hit': int(first_owned.sum()), 'accepted': int(accepted.sum()),
              'rejected_outside_mask': int(rejected.sum()), 'mask_uncovered': int(uncovered.sum()),
              'mask_uncovered_foreign_first_hit': int(foreign.sum()),
              'mask_uncovered_no_geometry': int(empty.sum()),
              'mask_pixels': int(acc.sum()), 'image': str(out)}
    (workspace / 'inspection/coverage-classes.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
