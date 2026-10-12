"""Lincoln painted-foliage inventory: tree/bush asset proposals and their source domains.

Plain Python (numpy/PIL/scipy/torch). Run from the repository root:

    python3 level-editor/blender/lincoln/trees_inventory.py

Lincoln foliage has no sight obstacles. Native occlusion masks exist for most clumps
(0-85, 153-161 and the garden layer-2 masks 327-338); the reviewed receiver manifest
(source-masks-v5) leaves them unassigned or uses them only as foreground exclusions.
A few clumps on the north ridge have no native mask at all and need authored domains.

Each proposal's *foliage domain* is its native mask union minus pixels that show
architecture through the (loose) native silhouette. That see-through carve uses a
small pixel classifier trained only on reviewed mask interiors far from class
boundaries; it is a proposal for visual review, never ownership authority.

Outputs (scratch, not frozen evidence) in work/needs-review/lincoln-refinement/scratch/trees/:
  inventory/tree-catalog-proposal.json   per-asset ids, names, masks, footprints, domains
  inventory/domains/<id>.png             full-image-box domain bitmaps (255 = foliage)
  inventory/overview.png                 annotated map overview
  inventory/ground-domain-trim.json      per terrain-receiver mask, pixels proposed to move
  inventory/architecture-seethrough.json per asset pixels proposed to return to receivers
"""
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage as nd

ROOT = Path(__file__).resolve().parents[3]
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
OUT = R / 'scratch/trees/inventory'
SOURCE = R / 'source-states/covered.png'
MASKS = R / 'mask-review/source-masks-v5.json'
INVENTORY = R / 'mask-review/inventory-v5/manifest.json'
W, H = 2944, 2176
BEHIND_ARCHITECTURE = {53, 54, 156, 160}
GROUND_DOMAINS = list(range(433, 453)) + [454]  # authored ground/rock domains; 454 = terrain ground (v5)

# (id, name, kind, native masks, region, note). Grouping is visual: masks whose
# painted crowns read as one plant form one asset; duplicates (L0/L2 copies)
# are listed together. Regions follow the terrain lane's names.
ASSETS = [
    # Village and stream (west lowland)
    ('lincoln-tree-village-northwest', 'Village northwestern autumn tree', 'tree', [0, 3, 26], 'village', ''),
    ('lincoln-bush-village-northwest', 'Village northwestern round bush', 'bush', [1], 'village', 'Stands in front of the tree crown (mask 26).'),
    ('lincoln-bush-village-cottage-shrub', 'Village cottage-side shrub', 'bush', [2], 'village', 'Small; may stay terrain texture.'),
    ('lincoln-bush-field-fence-west', 'Field wattle-fence western bushes', 'bush', [4, 5], 'village', ''),
    ('lincoln-bush-field-fence-east', 'Field wattle-fence eastern bushes', 'bush', [6], 'village', 'Wattle fence 118 excludes mask 6.'),
    ('lincoln-bush-stream-west-bank', 'Stream western bank bush', 'bush', [7], 'village', ''),
    ('lincoln-bush-stream-central', 'Stream central bush thicket', 'bush', [8, 9, 10], 'village', ''),
    ('lincoln-bush-stream-north', 'Stream northern bush thicket', 'bush', [11, 12], 'village', ''),
    ('lincoln-tree-stream-west', 'Stream western spreading tree', 'tree', [13], 'village', ''),
    ('lincoln-bush-stream-west-front', 'Stream western front bush', 'bush', [14], 'village', ''),
    ('lincoln-bush-stream-west-edge', 'Stream western map-edge bushes', 'bush', [15, 16], 'village', ''),
    ('lincoln-bush-stump-bridge', 'Stone footbridge bush', 'bush', [17], 'village', ''),
    ('lincoln-bush-village-bridge-thicket', 'Village bridge southern thicket', 'bush', [18, 19, 20], 'village', ''),
    ('lincoln-bush-village-bridge-stump', 'Village bridge stump-side bush', 'bush', [21], 'village', 'Tree stump 141 excludes mask 21.'),
    ('lincoln-bush-village-bridge-north', 'Village bridge northern bush', 'bush', [22], 'village', ''),
    ('lincoln-bush-village-bridge-east', 'Village bridge eastern bush', 'bush', [23], 'village', ''),
    ('lincoln-bush-stream-bend', 'Stream bend bush', 'bush', [24], 'village', ''),
    ('lincoln-bush-stream-east-bank', 'Stream eastern bank bush', 'bush', [25], 'village', ''),
    ('lincoln-bush-west-edge-north', 'Western map-edge northern bush', 'bush', [27], 'village', 'Rock 127 excludes mask 27.'),
    ('lincoln-bush-west-edge-south', 'Western map-edge southern bush', 'bush', [28], 'village', ''),
    ('lincoln-tree-stream-north-west', 'Northern stream western autumn tree', 'tree', [38, 42, 43], 'north-stream', ''),
    ('lincoln-tree-stream-north-east', 'Northern stream eastern autumn tree', 'tree', [39, 40], 'north-stream', ''),
    ('lincoln-tree-north-ridge-west', 'North ridge western tree', 'tree', [41], 'north-ridge', ''),
    ('lincoln-bush-north-ridge-west', 'North ridge western bush', 'bush', [44], 'north-ridge', ''),
    ('lincoln-bush-round-tower-north', 'Round tower northern bush', 'bush', [45], 'north-ridge', ''),
    ('lincoln-bush-round-tower-northeast', 'Round tower northeastern bush', 'bush', [46], 'north-ridge', ''),
    ('lincoln-bush-north-rim-west', 'North bailey rim western bush', 'bush', [47], 'north-bailey-rim', ''),
    # Southwest cliffs and road
    ('lincoln-tree-southwest-autumn-thicket', 'Southwestern autumn thicket', 'tree', [29, 30, 32], 'southwest', ''),
    ('lincoln-tree-southwest-green', 'Southwestern green tree', 'tree', [31], 'southwest', ''),
    ('lincoln-bush-southwest-thicket-edge', 'Southwestern thicket edge bush', 'bush', [33], 'southwest', ''),
    ('lincoln-bush-southwest-road', 'Southwestern road bush', 'bush', [34], 'southwest', ''),
    ('lincoln-bush-southwest-road-pair', 'Southwestern road bush pair', 'bush', [35], 'southwest', ''),
    ('lincoln-bush-southwest-cliff-foot', 'Southwestern cliff-foot bush', 'bush', [36], 'southwest', ''),
    ('lincoln-bush-west-edge-hillside', 'Western map-edge hillside bush', 'bush', [37], 'west-hillside', ''),
    ('lincoln-bush-southwest-cliff-upper', 'Southwestern cliff upper bush', 'bush', [61], 'southwest', ''),
    ('lincoln-bush-southwest-cliff-lower', 'Southwestern cliff lower bush', 'bush', [62], 'southwest', ''),
    ('lincoln-bush-southwest-cliff-east', 'Southwestern cliff eastern bush', 'bush', [63], 'southwest', ''),
    ('lincoln-bush-west-hillside-rock', 'Western hillside rock bush', 'bush', [64], 'west-hillside', ''),
    ('lincoln-bush-southwest-cliff-ledge', 'Southwestern cliff ledge bush', 'bush', [65], 'southwest', ''),
    ('lincoln-bush-ravine-bridge', 'Ravine bridge small bush', 'bush', [66], 'southwest', ''),
    ('lincoln-bush-south-bank', 'Southern bank bush', 'bush', [60], 'south-bank', ''),
    ('lincoln-tree-west-hillside', 'Western hillside tree', 'tree', [74], 'west-hillside', ''),
    ('lincoln-bush-west-hillside', 'Western hillside bush pair', 'bush', [72, 73], 'west-hillside', ''),
    ('lincoln-bush-west-tower-north', 'Western tower northern bush', 'bush', [83], 'west-hillside', ''),
    ('lincoln-bush-west-tower-northwest', 'Western tower northwestern bush', 'bush', [84], 'west-hillside', ''),
    ('lincoln-tree-west-outer-bailey', 'Western outer bailey autumn tree', 'tree', [154], 'inner-bailey', ''),
    # Castle garden (layer-2 masks)
    ('lincoln-tree-garden-west', 'Garden western small tree', 'tree', [335], 'garden', ''),
    ('lincoln-tree-garden-east', 'Garden eastern small tree', 'tree', [336], 'garden', ''),
    ('lincoln-bush-garden-bed', 'Garden wall flower bed', 'bush', [327, 328, 333], 'garden', ''),
    ('lincoln-bush-garden-northwest', 'Garden northwestern shrub', 'bush', [334, 338], 'garden', 'Masks 334 (L2) and 338 (L0) are identical.'),
    ('lincoln-bush-garden-southwest', 'Garden southwestern shrubs', 'bush', [329, 332], 'garden', ''),
    # Castle inner areas
    ('lincoln-tree-behind-great-hall', 'Tree behind the great hall', 'tree', [160], 'north-bailey', 'Mask 160 is a loose envelope over the hall roofs; only the crown above the roof is foliage.'),
    ('lincoln-bush-north-curtain-west', 'North curtain western bush', 'bush', [158], 'north-bailey', ''),
    ('lincoln-tree-north-curtain-west', 'North curtain western tree', 'tree', [159], 'north-bailey', ''),
    ('lincoln-tree-north-bailey-rim', 'North bailey rim autumn tree', 'tree', [48, 49], 'north-bailey-rim', 'Stands in front of the north curtain; excluded from the curtain receiver.'),
    ('lincoln-tree-north-bailey-autumn', 'North bailey large autumn tree', 'tree', [78, 85], 'north-bailey', 'Masks 78 (L0) and 85 (L2) are identical.'),
    ('lincoln-tree-north-bailey-east', 'North bailey eastern tree', 'tree', [77, 157, 161], 'north-bailey', 'Masks 157 (L0) and 161 (L2) are identical; 77 is its lower foliage.'),
    ('lincoln-bush-north-bailey-crates', 'North bailey crate-side bush', 'bush', [82], 'north-bailey', ''),
    ('lincoln-bush-north-hall', 'North hall bush', 'bush', [80], 'north-bailey', ''),
    ('lincoln-bush-keep-north', 'Keep northern bush', 'bush', [81], 'north-bailey', ''),
    ('lincoln-bush-bailey-cottage-west', 'Inner bailey cottage western shrub', 'bush', [67], 'inner-bailey', ''),
    ('lincoln-bush-bailey-shed', 'Inner bailey shed bush', 'bush', [68], 'inner-bailey', ''),
    ('lincoln-bush-bailey-rock', 'Inner bailey rock-spur bushes', 'bush', [69], 'inner-bailey', ''),
    ('lincoln-bush-south-curtain-stair', 'Southern curtain stair bush', 'bush', [153], 'inner-bailey', ''),
    # North outer plateau and ridge (beyond the curtain)
    ('lincoln-tree-north-plateau-west', 'North outer plateau western autumn tree', 'tree', [50, 52], 'north-outer-plateau', ''),
    ('lincoln-tree-north-plateau-red', 'North outer plateau red autumn tree', 'tree', [51], 'north-outer-plateau', ''),
    ('lincoln-tree-north-plateau-east', 'North outer plateau large eastern tree', 'tree', [53], 'north-outer-plateau', 'Excluded from the NE tower receiver as background by v1 review.'),
    ('lincoln-tree-northeast-forest-west', 'Northeastern forest western tree', 'tree', [56], 'north-outer-plateau', ''),
    ('lincoln-tree-northeast-forest-edge', 'Northeastern forest map-edge tree', 'tree', [55], 'north-outer-plateau', ''),
    ('lincoln-tree-northeast-forest-south', 'Northeastern forest southern tree', 'tree', [54], 'north-outer-plateau', 'Excluded from the NE spire receiver as background by v1 review.'),
    # East slope
    ('lincoln-bush-northeast-tower-stair', 'Northeastern tower stair bush', 'bush', [70], 'east-slope', ''),
    ('lincoln-bush-northeast-curtain-foot', 'Northeastern curtain foot bush', 'bush', [71], 'east-slope', ''),
    ('lincoln-tree-northeast-curtain', 'Northeastern curtain autumn tree', 'tree', [156], 'east-slope', ''),
    ('lincoln-bush-east-slope-north', 'Eastern slope northern bushes', 'bush', [76, 155], 'east-slope', ''),
    ('lincoln-bush-east-slope-edge', 'Eastern slope map-edge bush', 'bush', [79], 'east-slope', ''),
    ('lincoln-bush-east-slope-middle', 'Eastern slope middle bush', 'bush', [59], 'east-slope', ''),
    ('lincoln-bush-east-field-edge', 'Eastern field map-edge bush', 'bush', [57], 'east-slope', ''),
    ('lincoln-tree-east-field', 'Eastern field lone tree', 'tree', [58], 'east-slope', ''),
    ('lincoln-bush-east-slope-rock', 'Eastern slope rock bush', 'bush', [75], 'east-slope', ''),
]

# Foliage without any native mask: reviewed source boxes; the domain inside each
# box is the foliage classifier's output minus architecture and native foliage.
UNMASKED = [
    ('lincoln-tree-north-ridge-bare', 'North ridge bare-branched tree', 'tree', (2470, 0, 2640, 100), 'north-ridge',
     'No native mask; bare branches with sparse autumn leaves at the top map edge.'),
    ('lincoln-bush-north-ridge-autumn', 'North ridge autumn bush', 'bush', (2420, 70, 2530, 165), 'north-ridge',
     'No native mask.'),
    ('lincoln-bush-north-ridge-green', 'North ridge dark green bushes', 'bush', (2600, 60, 2700, 240), 'north-ridge',
     'No native mask; continues east into mask 56.'),
    ('lincoln-tree-north-ridge-edge', 'North ridge map-edge autumn tree', 'tree', (2170, 0, 2290, 55), 'north-ridge',
     'No native mask; crown cut by the map top edge.'),
    # Reported by the terrain-ground worker as unmasked foliage inside its ground domain.
    ('lincoln-bush-stream-north-west-bank', 'Northern stream western bank bush', 'bush', (1090, 120, 1150, 155), 'village',
     'No native mask; ground-domain carve requested by the terrain-ground worker.'),
    ('lincoln-bush-stream-north-east-bank', 'Northern stream eastern bank shrub', 'bush', (1290, 145, 1350, 190), 'village',
     'No native mask; ground-domain carve requested by the terrain-ground worker.'),
    ('lincoln-bush-stream-bend-small', 'Stream bend small shrub', 'bush', (1195, 465, 1225, 495), 'village',
     'No native mask; ground-domain carve requested by the terrain-ground worker.'),
    ('lincoln-bush-pond-south', 'Pond southern bush', 'bush', (400, 842, 445, 880), 'village',
     'No native mask; ground-domain carve requested by the terrain-ground worker.'),
    ('lincoln-bush-stream-west-fern', 'Western stream fern tuft', 'bush', (40, 960, 80, 990), 'village',
     'No native mask; ground-domain carve requested by the terrain-ground worker.'),
]


# Obstacle-less non-foliage scenery: painted pixels in the box that no reviewed
# receiver owns (the terrain-ground lane carved them out of ground mask 454).
SCENERY = [
    ('lincoln-village-pond-landing-stage', 'Village pond landing stage and mooring pole', 'scenery', (484, 612, 548, 678),
     'village', 'No node or native mask; plank deck on posts plus a mooring pole. The punt belongs to lincoln-village-stream-boat.'),
]
# Traced source-pixel outlines (x, y) of the painted landing stage: deck with its
# shaded left face, the front-left post, and the mooring pole at the front-right corner.
SCENERY_POLYGONS = {
    'lincoln-village-pond-landing-stage': [
        [(486, 630), (529, 621), (543, 648), (506, 662), (497, 652)],
        [(505, 660), (511, 660), (511, 678), (505, 678)],
        [(540, 614), (545, 614), (545, 673), (540, 673)],
    ],
}


# Non-foliage artwork inside native foliage masks, traced on the source (x, y).
DOMAIN_EXCLUSIONS = {
    # Boulder behind the west plateau tree (mask 50/52 envelope); it is north-rock-ridge terrain.
    'lincoln-tree-north-plateau-west': [[(2377, 257), (2405, 255), (2428, 266), (2433, 274), (2423, 282),
                                        (2407, 285), (2392, 290), (2378, 285), (2375, 272)]],
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_inventory():
    records = {m['index']: m for m in json.loads(INVENTORY.read_text())['masks']}
    def full(index):
        record = records[index]
        bitmap = np.asarray(Image.open(INVENTORY.parent / record['png']).convert('L')) > 0
        x, y = record['box_top_left']
        out = np.zeros((H, W), bool)
        h, w = bitmap.shape
        out[y:y + h, x:x + w] = bitmap[:H - y, :W - x]
        return out
    return records, full


def reviewed_masks(full):
    manifest = json.loads(MASKS.read_text())
    arch = np.zeros((H, W), bool)
    ground = {}
    owners = {}
    entries = [e for p in manifest['projections'].values() for e in p['assignments']]
    # Terrain receivers: every node that owns an authored ground domain. Their
    # native rock masks (e.g. 264) are terrain too, so foliage may be trimmed from them.
    terrain_nodes = {e['source_node'] for e in entries if set(e['mask_indices']) & set(GROUND_DOMAINS)}
    for entry in entries:
        for index in entry['mask_indices']:
            owners.setdefault(index, []).append(entry['source_node'])
    for entry in entries:
        for index in entry['mask_indices']:
            if index == 428:
                continue
            if entry['source_node'] in terrain_nodes and all(o in terrain_nodes for o in owners[index]):
                if index not in ground:
                    ground[index] = full(index)
            else:
                arch |= full(index)
    return manifest, arch, ground, owners


def pixel_classifier(source, positive, architecture, ground):
    """Three-way (foliage/architecture/ground) MLP on local colour/texture features."""
    import torch
    torch.manual_seed(0)
    rng = np.random.default_rng(0)
    rgb = source.astype(np.float32) / 255
    lum = rgb @ np.array([.3, .59, .11], np.float32)
    top, bottom = rgb.max(-1), rgb.min(-1)
    features = [rgb, ((top - bottom) / (top + 1e-6))[..., None]]
    for size in (2, 5, 11):
        features.append(np.stack([nd.uniform_filter(rgb[..., c], size) for c in range(3)], -1))
        variance = nd.uniform_filter(lum * lum, size) - nd.uniform_filter(lum, size) ** 2
        features.append(np.sqrt(np.clip(variance, 0, None))[..., None])
    features.append(nd.uniform_filter(np.hypot(nd.sobel(lum, 1), nd.sobel(lum, 0)), 5)[..., None])
    table = np.concatenate(features, -1).reshape(-1, 17).astype(np.float32)
    away_arch = nd.distance_transform_edt(~architecture) >= 6
    away_foliage = nd.distance_transform_edt(~positive) >= 6
    classes = [positive & away_arch, architecture & away_foliage, ground & away_foliage & away_arch]
    picks = [rng.choice(np.flatnonzero(c), min(120000, int(c.sum())), replace=False) for c in classes]
    x = torch.tensor(np.concatenate([table[p] for p in picks]))
    y = torch.tensor(np.concatenate([np.full(len(p), k) for k, p in enumerate(picks)]))
    mean, std = x.mean(0), x.std(0) + 1e-6
    net = torch.nn.Sequential(torch.nn.Linear(17, 64), torch.nn.ReLU(), torch.nn.Linear(64, 64),
                              torch.nn.ReLU(), torch.nn.Linear(64, 3))
    optimiser = torch.optim.Adam(net.parameters(), 1e-3)
    for _ in range(12):
        order = torch.randperm(len(x))
        for k in range(0, len(order), 4096):
            batch = order[k:k + 4096]
            loss = torch.nn.functional.cross_entropy(net((x[batch] - mean) / std), y[batch])
            optimiser.zero_grad(); loss.backward(); optimiser.step()
    with torch.no_grad():
        chunks = [torch.softmax(net((torch.tensor(table[k:k + 500000]) - mean) / std), 1)
                  for k in range(0, len(table), 500000)]
    return torch.cat(chunks).numpy().reshape(H, W, 3)


def clean(mask, minimum=40):
    labels, count = nd.label(mask)
    if not count:
        return mask
    sizes = nd.sum(mask, labels, range(1, count + 1))
    return np.isin(labels, np.flatnonzero(sizes >= minimum) + 1)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'domains').mkdir(exist_ok=True)
    source = np.asarray(Image.open(SOURCE).convert('RGB'))
    records, full = load_inventory()
    manifest, arch, ground, owners = reviewed_masks(full)
    ground_union = np.zeros((H, W), bool)
    for bitmap in ground.values():
        ground_union |= bitmap
    native = {index: full(index) for asset in ASSETS for index in asset[3]}
    foliage_union = np.zeros((H, W), bool)
    for bitmap in native.values():
        foliage_union |= bitmap
    cache = OUT / 'classifier.npy'
    if cache.exists() and '--retrain' not in sys.argv:
        proba = np.load(cache).astype(np.float32)
    else:
        proba = pixel_classifier(source, foliage_union, arch, ground_union)
        np.save(cache, proba.astype(np.float16))
    architecture_like = proba[..., 1] > .5
    foliage_like = proba[..., 0] > proba[..., 2]
    assigned = np.zeros((H, W), np.int16)
    rows = []
    trims = {index: np.zeros((H, W), bool) for index in ground}
    for number, (asset_id, name, kind, masks, region, note) in enumerate(ASSETS + UNMASKED + SCENERY, 1):
        if kind == 'scenery':
            x0, y0, x1, y1 = masks
            box = np.zeros((H, W), bool)
            box[y0:y1, x0:x1] = True
            from PIL import ImageDraw
            traced = Image.new('L', (W, H), 0)
            for polygon in SCENERY_POLYGONS[asset_id]:
                ImageDraw.Draw(traced).polygon(polygon, fill=255)
            # Within the unowned hole the terrain lane carved, keep only the traced stage.
            domain = box & (np.asarray(traced) > 0) & ~arch & ~ground_union & ~foliage_union
            envelope = domain
            seethrough = np.zeros((H, W), bool)
            native_masks = []
            source_kind = 'traced-polygon-within-unowned-pixels'
        elif isinstance(masks, tuple):
            x0, y0, x1, y1 = masks
            box = np.zeros((H, W), bool)
            box[y0:y1, x0:x1] = True
            domain = box & foliage_like & ~arch & ~foliage_union
            domain = nd.binary_closing(nd.binary_opening(domain, iterations=1), iterations=2) & box & ~arch & ~foliage_union
            domain = clean(domain, min(200, int(.15 * (x1 - x0) * (y1 - y0))))
            envelope = domain
            seethrough = np.zeros((H, W), bool)
            native_masks = []
            source_kind = 'authored-box-classifier'
        else:
            envelope = np.zeros((H, W), bool)
            for index in masks:
                envelope |= native[index]
            # v1 review: these crowns stand behind the architecture they overlap, so
            # every reviewed architecture pixel stays with its receiver.
            behind = set(masks) & BEHIND_ARCHITECTURE
            seethrough = envelope & arch & (architecture_like | bool(behind))
            seethrough = nd.binary_closing(nd.binary_opening(seethrough, iterations=1), iterations=2) & envelope & arch
            seethrough = clean(seethrough)
            domain = envelope & ~seethrough
            # Drop isolated speckles far from any crown body (loose native masks
            # scatter leaf dots over paving); leaf-edge dots next to the crown stay.
            closed = nd.binary_closing(domain, iterations=3)
            labels, count = nd.label(closed)
            sizes = nd.sum(closed, labels, range(1, count + 1)) if count else np.zeros(0)
            keep = np.flatnonzero(sizes >= max(300, .05 * sizes.max())) + 1 if count else []
            bodies = np.isin(labels, keep)
            seethrough |= domain & ~bodies
            domain &= bodies
            native_masks = list(masks)
            source_kind = 'native-mask-union-minus-architecture-seethrough'
        if asset_id in DOMAIN_EXCLUSIONS:
            from PIL import ImageDraw
            traced = Image.new('L', (W, H), 0)
            for polygon in DOMAIN_EXCLUSIONS[asset_id]:
                ImageDraw.Draw(traced).polygon(polygon, fill=255)
            cut = domain & (np.asarray(traced) > 0)
            seethrough |= cut
            domain &= ~cut
        overlap = (assigned > 0) & domain
        # Domains may share pixels (one crown in front of another). Both keep them;
        # fixed-camera first-hit gating on the fitted crowns decides the receiver.
        # `assigned` only colours the overview.
        shares = sorted({rows[i - 1]['id'] for i in np.unique(assigned[overlap]) if i})
        owned = domain & (assigned == 0)
        assigned[owned] = number
        ys, xs = np.nonzero(domain)
        if not len(xs):
            raise ValueError(f'Empty foliage domain: {asset_id}')
        box = [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1]
        bottom = int(ys.max())
        base_row = xs[ys >= bottom - 6]
        crop = domain[box[1]:box[3], box[0]:box[2]]
        path = OUT / 'domains' / f'{asset_id}.png'
        Image.fromarray((crop * 255).astype(np.uint8)).save(path)
        ground_hits = {}
        for index, bitmap in ground.items():
            hit = domain & bitmap
            if hit.any():
                ground_hits[index] = int(hit.sum())
                trims[index] |= hit
        rows.append(dict(
            id=asset_id, name=name, kind=kind, region=region, note=note,
            native_masks=native_masks, domain_method=source_kind,
            native_mask_layers={str(i): records[i]['layer'] for i in native_masks},
            native_mask_current_roles={str(i): owners.get(i, ['unassigned']) for i in native_masks},
            source_box=box, envelope_pixels=int(envelope.sum()), domain_pixels=int(domain.sum()),
            architecture_seethrough_pixels=int(seethrough.sum()),
            shared_pixels_first_hit_gated=int(overlap.sum()), shares_pixels_with=shares,
            ground_domain_pixels=ground_hits,
            base_pixel=[int(np.median(base_row)), bottom],
            domain_png=str(path), domain_sha256=sha(path), domain_box_top_left=box[:2]))
    catalog = dict(
        version=1, map='lincoln', status='proposal-not-reviewed',
        source=str(SOURCE), source_sha256=sha(SOURCE),
        mask_manifest=str(MASKS), mask_manifest_sha256=sha(MASKS),
        mask_inventory=str(INVENTORY), mask_inventory_sha256=sha(INVENTORY),
        counts=dict(assets=len(rows), trees=sum(r['kind'] == 'tree' for r in rows),
                    bushes=sum(r['kind'] == 'bush' for r in rows), scenery=sum(r['kind'] == 'scenery' for r in rows),
                    with_native_masks=sum(bool(r['native_masks']) for r in rows),
                    authored_only=sum(not r['native_masks'] for r in rows),
                    native_masks_used=len(native)),
        method=('Native foliage masks grouped visually into selectable plants. Domain = mask union minus '
                'architecture see-through pixels (classifier proposal). Unmasked clumps use reviewed boxes and '
                'the same classifier. Domains may overlap (nested crowns); both assets keep shared pixels and '
                'fixed-camera first-hit gating on the fitted crown geometry resolves them.'),
        retained_as_terrain=[
            'Ivy and creeper painted on masonry (keep, north curtain, east curtain, NE tower) belongs to the wall receivers.',
            'Ground-hugging shrub carpet on the inner rock spur (1140-1240, 1140-1220) stays rock/terrain texture.',
            'Grass tufts and low shrubs on the southwest and southeast cliffs without native masks stay terrain texture.'],
        assets=rows)
    (OUT / 'tree-catalog-proposal.json').write_text(json.dumps(catalog, indent=2) + '\n')
    trim_rows = []
    for index, bitmap in sorted(trims.items()):
        if not bitmap.any():
            continue
        path = OUT / f'ground-trim-{index}.png'
        record = records[index]
        x, y = record['box_top_left']
        w, h = record['box_size']
        Image.fromarray((bitmap[y:y + h, x:x + w] * 255).astype(np.uint8)).save(path)
        trim_rows.append(dict(ground_mask=index, ground_mask_png=record['png'], owner=owners.get(index),
                              remove_pixels=int(bitmap.sum()), ground_pixels=int(ground[index].sum()),
                              trim_png=str(path), trim_sha256=sha(path), trim_box_top_left=[x, y]))
    (OUT / 'ground-domain-trim.json').write_text(json.dumps(dict(
        version=1, status='proposal-not-reviewed',
        rule=('Remove these pixels (union of proposed foliage domains) from each terrain-receiver mask in masks v5: '
              'authored domains 433-452 directly; native rock masks via a new exclusion of the tree domains.'),
        trims=trim_rows), indent=2) + '\n')
    (OUT / 'architecture-seethrough.json').write_text(json.dumps(dict(
        version=1, status='proposal-not-reviewed',
        rule=('Native foliage-mask pixels left out of the proposed domain: masonry/roof seen through the loose '
              'silhouette (all architecture pixels for crowns behind architecture: masks 53, 54, 156, 160) and '
              'detached leaf specks. Receivers that exclude the whole native mask lose this artwork; v5 could '
              'exclude the foliage domain instead.'),
        assets={r['id']: r['architecture_seethrough_pixels'] for r in rows if r['architecture_seethrough_pixels']}),
        indent=2) + '\n')
    overview(source, rows, assigned, arch)
    print(json.dumps(catalog['counts']))


def overview(source, rows, assigned, arch):
    image = source.astype(np.float32) * .45
    rng = np.random.default_rng(3)
    colours = rng.integers(60, 255, size=(len(rows) + 1, 3)).astype(np.float32)
    mask = assigned > 0
    image[mask] = source[mask] * .35 + colours[assigned[mask]] * .65
    edge = mask & ~nd.binary_erosion(mask)
    image[edge] = 255
    picture = Image.fromarray(image.clip(0, 255).astype(np.uint8))
    draw = ImageDraw.Draw(picture)
    try:
        font = ImageFont.truetype('DejaVuSans-Bold.ttf', 15)
    except OSError:
        font = ImageFont.load_default()
    for number, row in enumerate(rows, 1):
        x0, y0, x1, y1 = row['source_box']
        draw.rectangle((x0, y0, x1, y1), outline=tuple(int(c) for c in colours[number]), width=2)
        bx, by = row['base_pixel']
        draw.ellipse((bx - 4, by - 4, bx + 4, by + 4), fill=(255, 0, 0))
        label = f"{number} {row['id'].removeprefix('lincoln-')}"
        draw.rectangle((x0, y0, x0 + 8 * len(label), y0 + 17), fill=(0, 0, 0))
        draw.text((x0 + 2, y0 + 1), label, fill=(255, 255, 255), font=font)
    picture.save(OUT / 'overview.png')
    picture.resize((W // 2, H // 2), Image.LANCZOS).save(OUT / 'overview-half.png')


if __name__ == '__main__':
    main()
