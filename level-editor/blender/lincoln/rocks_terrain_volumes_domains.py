"""Author reviewed ground/rock receiver domains for the rocks/terrain lane (system python).

    python3 level-editor/blender/lincoln/rocks_terrain_volumes_domains.py <firsthit.npz> [--write]

Domain(node) = pixels of covered.png where the node is the first-hit surface
of the integrated scene at the source camera, minus every scenery silhouette
(all covered-state native masks except the terrain silhouettes and masks
owned only by this lane, plus the authored walkway/deck domains), minus the
manual carve-outs in rocks_terrain_volumes_carveouts.py (omitted painted
props found by inspection). Pixels the node already accepts from its reviewed
native include masks are reported separately (the domain may overlap them).

With --write each node's domain goes to <workspace>/inspection/ground-domain-<node>.{png,json}
plus an evidence sheet; otherwise only statistics are printed.
"""
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from rocks_terrain_volumes_carveouts import CARVEOUTS, MIN_PIXELS, SKIP  # noqa: E402

ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
SOURCE = ROOT / 'source-states/covered.png'
MANIFEST = ROOT / 'mask-review/source-masks-v3.json'
CATALOG = ROOT / 'grouping/catalog-v3.json'
LANE = 'rocks_terrain_volumes'
TERRAIN_SILHOUETTES = set(range(259, 270))


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def workspace(asset):
    overrides = json.loads((ROOT / 'workspace-overrides.json').read_text())['assets']
    if asset in overrides:
        return Path(overrides[asset])
    return ROOT / 'round-3/assets' / asset


def main():
    npz = np.load(sys.argv[1], allow_pickle=True)
    write = '--write' in sys.argv
    label, names = npz['label'], list(npz['names'])
    catalog = json.loads(CATALOG.read_text())
    lane_assets = json.loads((ROOT / 'worker-assignments.json').read_text())['assignments'][LANE]
    node_asset = {}
    for g in catalog['groups']:
        if g['id'] in lane_assets:
            for p in g['parts']:
                node_asset[f"building-{p['obstacle']:03d}"] = g['id']
    manifest = json.loads(MANIFEST.read_text())
    inv_path = (MANIFEST.parent / manifest['mask_inventory']).resolve()
    inventory = json.loads(inv_path.read_text())
    records = {r['index']: r for r in inventory['masks']}
    sys.path.insert(0, str(ROOT / 'mask-review'))
    layers = json.loads((ROOT / 'source-states/layers.json').read_text())
    by_layer = {(r['layer'], r['layer_index']): r['index'] for r in inventory['masks'] if r.get('layer') is not None}
    initial, applied = set(), set()
    for patch in layers['patches']:
        initial |= {by_layer[(m['layer'], m['index'])] for m in patch['state']['old_masks']}
        applied |= {by_layer[(m['layer'], m['index'])] for m in patch['state']['new_masks']}
    applied_only = applied - initial
    rows = {r['source_node']: r for r in manifest['projections']['exterior']['assignments'] if 'source_node' in r}
    owners = {}
    for node, row in rows.items():
        for i in row['mask_indices'] + row.get('exclude_mask_indices', []):
            owners.setdefault(i, set()).add(node)
    lane_only = {i for i, nodes in owners.items() if nodes <= set(node_asset)}
    scenery_idx = []
    for i, r in records.items():
        if r.get('png') is None or i == 428:
            continue
        if r.get('synthetic'):
            if not (owners.get(i, set()) <= set(node_asset)):
                scenery_idx.append(i)
            continue
        if i in applied_only or i in TERRAIN_SILHOUETTES or i in lane_only:
            continue
        scenery_idx.append(i)

    def bitmap(i):
        r = records[i]
        a = np.array(Image.open(inv_path.parent / r['png']).convert('L')) > 0
        full = np.zeros((2176, 2944), bool)
        x, y = r['box_top_left']
        h, w = a.shape
        full[max(0, y):min(2176, y + h), max(0, x):min(2944, x + w)] = \
            a[max(0, -y):min(h, 2176 - y), max(0, -x):min(w, 2944 - x)]
        return full

    scenery = np.zeros((2176, 2944), bool)
    for i in scenery_idx:
        scenery |= bitmap(i)
    source = Image.open(SOURCE).convert('RGB')
    src = np.array(source)
    report = {}
    for node, asset in sorted(node_asset.items()):
        if node in SKIP:
            report[node] = {'asset': asset, 'skipped': SKIP[node]}
            continue
        ids = [k for k, n in enumerate(names) if n == node]
        first = np.isin(label, ids) if ids else np.zeros_like(scenery)
        carve = np.zeros_like(scenery)
        for poly in CARVEOUTS.get(node, []):
            img = Image.new('1', (2944, 2176), 0)
            ImageDraw.Draw(img).polygon([tuple(p) for p in poly['polygon']], fill=1)
            carve |= np.array(img, bool)
        domain = first & ~scenery & ~carve
        own = np.zeros_like(scenery)
        for i in rows[node]['mask_indices']:
            if i != 428:
                own |= bitmap(i)
        for i in rows[node].get('exclude_mask_indices', []):
            own &= ~bitmap(i)
        new = domain & ~own
        stats = {'asset': asset, 'first_hit': int(first.sum()), 'scenery_removed': int((first & scenery).sum()),
                 'carved': int((first & carve & ~scenery).sum()), 'domain': int(domain.sum()),
                 'already_own_mask': int((domain & own).sum()), 'new_pixels': int(new.sum())}
        report[node] = stats
        if not write or stats['new_pixels'] < MIN_PIXELS:
            continue
        ws = workspace(asset)
        out = ws / 'inspection'
        out.mkdir(exist_ok=True)
        ys, xs = np.nonzero(domain)
        x0, y0, x1, y1 = int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1
        crop = domain[y0:y1, x0:x1]
        png = out / f'ground-domain-{node}.png'
        Image.fromarray((crop * 255).astype(np.uint8)).save(png)
        # Evidence: unmarked | domain (cyan) with scenery (magenta) and carve-outs (orange).
        p = 30
        bx0, by0, bx1, by1 = max(0, x0 - p), max(0, y0 - p), min(2944, x1 + p), min(2176, y1 + p)
        base = src[by0:by1, bx0:bx1].astype(float)
        vis = base * 0.55
        sel = domain[by0:by1, bx0:bx1]
        vis[sel] = base[sel] * 0.5 + np.array([0, 230, 230]) * 0.5
        sc = scenery[by0:by1, bx0:bx1] & first[by0:by1, bx0:bx1]
        vis[sc] = base[sc] * 0.5 + np.array([230, 0, 230]) * 0.5
        cv = carve[by0:by1, bx0:bx1] & first[by0:by1, bx0:bx1]
        vis[cv] = base[cv] * 0.4 + np.array([255, 140, 0]) * 0.6
        sheet = np.concatenate([base, np.full((base.shape[0], 6, 3), 30.0), vis], 1).astype(np.uint8)
        img = Image.fromarray(sheet)
        scale = min(1.0, 2400 / img.width)
        if scale < 1:
            img = img.resize((int(img.width * scale), int(img.height * scale)))
        evidence = out / f'ground-domain-{node}-evidence.png'
        img.save(evidence)
        record = {
            'version': 1, 'asset_id': asset, 'source_node': node,
            'purpose': ('Authored receiver domain for the painted ground/rock surface of this terrain node '
                        '(no native mask outlines walkable ground or rock faces). Proposed for mask inventory v4 '
                        'as a reviewed synthetic mask assigned to this node with first-hit gating.'),
            'source': str(SOURCE), 'source_sha256': sha(SOURCE),
            'coordinates': 'original full-image covered.png pixels (x right, y down)',
            'derivation': {
                'first_hit': ('Pixels where this node is the front-most surface of the integrated scene '
                              '(round-3 bridge workspace model: grouped v3 + round-3 lane geometry) at the '
                              'source camera (native (x, y, z) -> pixel (x, y - z)); z-buffer by '
                              'rocks_terrain_volumes_firsthit.py.'),
                'scenery_removed': (f'Minus every covered-state native silhouette not owned solely by this lane '
                                    f'({len(scenery_idx)} masks: buildings, walls, props, trees, bushes, and the '
                                    'authored walkway/deck domains). Terrain silhouettes 259-269 and lane rock '
                                    'masks are kept. Patch applied-only masks are ignored.'),
                'carveouts': CARVEOUTS.get(node, []),
                'shadows': 'Shadows cast on the ground by structures are painted ground and stay in the domain.',
                'unmasked_foliage': ('Bushes and tree canopies painted over the ground without any native '
                                     'silhouette (mainly north outer plateau 068, north bailey 067 rim, east slope '
                                     '064) stay in the domain: the ground is their only possible receiver. They '
                                     'are flagged for texture review, not carved.'),
                'gating': 'First-hit receiver gating stays mandatory; the domain never widens a neighbour.'},
            'bitmap': {'png': str(png.relative_to(ws)), 'box_top_left': [x0, y0], 'box_size': [x1 - x0, y1 - y0],
                       'pixel_values': '0 = outside, 255 = ground domain (same convention as inventory-v1)',
                       'pixels': int(domain.sum()), 'sha256': sha(png)},
            'evidence': {'annotated': str(evidence.relative_to(ws)), 'annotated_sha256': sha(evidence),
                         'legend': 'left unmarked covered.png; right cyan domain, magenta removed scenery, '
                                   'orange manual carve-outs'},
            'statistics': stats,
            'scenery_mask_indices': sorted(scenery_idx),
            'first_hit_scene': 'round-3/assets/lincoln-southwest-ravine-bridge/model.blend',
            'first_hit_npz_sha256': sha(sys.argv[1]),
        }
        (out / f'ground-domain-{node}.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(report, indent=1))


if __name__ == '__main__':
    main()
