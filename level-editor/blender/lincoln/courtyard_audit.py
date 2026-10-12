"""Source-coverage audit for the Lincoln courtyard lane (pure Python).

For one workspace this compares the saved model's source-camera render
(modified view 0: azimuth 0, 35 degrees, orthographic, i.e. the artwork's own
camera) with the original artwork, independently of the acceptance masks:

* audit domain D  = every view-0 pixel covered by the asset's saved geometry
  (solid render alpha), mapped back to artwork pixels;
* own artwork M   = the asset's reviewed native masks from the working
  source-masks.json (reject-all nodes contribute nothing);
* accepted T      = view-0 known pixels (the packet's own ownership result).

It reports accepted/rejected pixels inside D, own-mask pixels not covered by
geometry (missing receivers), RGB preservation of accepted pixels, and names
the foreign native masks that own neutral pixels. Evidence is a side-by-side
image of artwork, textured render and classification map.

Usage: python3 courtyard_audit.py <workspace> [--write]
"""
import hashlib
import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

S = math.sin(math.radians(35))
C = math.cos(math.radians(35))
LINCOLN = Path(__file__).resolve().parents[2] / 'work/needs-review/lincoln-refinement'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _mask_manifest():
    return {e['global_index']: e for e in
            json.loads((LINCOLN / 'source-states/mask-manifest.json').read_text())['masks']}


def _paint_mask(canvas, origin, entry):
    mk = np.array(Image.open(LINCOLN / 'source-states' / entry['image'])) > 0
    if mk.ndim == 3:
        mk = mk[..., -1]
    bx, by = entry['box_top_left']
    x0, y0 = origin
    h, w = canvas.shape
    ys, xs = np.nonzero(mk)
    X, Y = xs + bx - x0, ys + by - y0
    keep = (X >= 0) & (X < w) & (Y >= 0) & (Y < h)
    canvas[Y[keep], X[keep]] = True


def audit(workspace, first_hit=None):
    """`first_hit(points)` (Blender only) maps view-0 pixel centres to the first
    object hit from the source camera, attributing rejected own artwork."""
    workspace = Path(workspace).resolve()
    config = json.loads((workspace / 'workspace.json').read_text())
    views = json.loads((workspace / 'modified/views.json').read_text())
    view = views['views'][0]
    if view['azimuth_degrees'] != 0 or abs(views['elevation_degrees'] - 35) > 1e-6:
        raise ValueError('View 0 is not the source camera')
    width, height = views['tile_size']
    k = max(width, height) / view['ortho_scale']
    cam = view['camera_location']
    cam_sx, cam_sy = cam[0], -(cam[1] * S + cam[2] * C)
    solid = np.array(Image.open(workspace / 'modified/views/view-0-solid.png'))
    known = np.array(Image.open(workspace / 'modified/views/view-0-known.png'))[..., 0] > 127
    textured = np.array(Image.open(workspace / 'modified/views/view-0-textured.png'))[..., :3].astype(int)
    domain = solid[..., 3] > 0
    vy, vx = np.mgrid[0:height, 0:width]
    sx = np.floor((vx + 0.5 - width / 2) / k + cam_sx).astype(int)
    sy = np.floor((vy + 0.5 - height / 2) / k + cam_sy).astype(int)
    source = np.array(Image.open(LINCOLN / 'source-states/covered.png').convert('RGB')).astype(int)
    if sha(LINCOLN / 'source-states/covered.png') != views['source_sha256']:
        raise ValueError('Packet source differs from covered.png')
    x0, y0 = int(sx[domain].min()) - 30, int(sy[domain].min()) - 30
    x1, y1 = int(sx[domain].max()) + 31, int(sy[domain].max()) + 31
    shape = (y1 - y0, x1 - x0)
    manifest = _mask_manifest()
    masks = json.loads(Path(config['source_mask_manifest']).read_text())
    rows = masks['projections']['exterior']['assignments']
    own_nodes = set(config['part_ids'])
    own_indices = sorted({i for r in rows if r['source_node'] in own_nodes for i in r['mask_indices']} - {428})
    rejected_nodes = sorted(r['source_node'] for r in rows if r['source_node'] in own_nodes and r['mask_indices'] == [428])
    # Own artwork: per receiver row, its masks minus its reviewed foreground
    # exclusions (the same rule the projection applies).
    own = np.zeros(shape, bool)
    excluded_indices = set()
    for r in rows:
        if r['source_node'] not in own_nodes or r['mask_indices'] == [428]:
            continue
        row_mask = np.zeros(shape, bool)
        for index in r['mask_indices']:
            _paint_mask(row_mask, (x0, y0), manifest[index])
        for index in r.get('exclude_mask_indices', []):
            excl = np.zeros(shape, bool)
            _paint_mask(excl, (x0, y0), manifest[index])
            row_mask &= ~excl
            excluded_indices.add(index)
        own |= row_mask
    # Foreign native masks intersecting the crop (for naming neutral pixels).
    foreign_nodes = {i: {'reviewed foreground exclusion'} for i in excluded_indices}
    for r in rows:
        if r['source_node'] in own_nodes:
            continue
        for i in r['mask_indices']:
            if i != 428:
                foreign_nodes.setdefault(i, set()).add(r['source_node'])
    # Name at most three owners per mask in the report.
    foreign_nodes = {i: set(sorted(v)[:3]) | ({'...'} if len(v) > 3 else set()) for i, v in foreign_nodes.items()}
    foreign_layers = {}
    for index in foreign_nodes:
        e = manifest[index]
        bx, by = e['box_top_left']
        bw, bh = e['box_size']
        if bx < x1 and bx + bw > x0 and by < y1 and by + bh > y0:
            canvas = np.zeros(shape, bool)
            _paint_mask(canvas, (x0, y0), e)
            if canvas.any():
                foreign_layers[index] = canvas
    lx, ly = sx - x0, sy - y0
    inside = (lx >= 0) & (lx < shape[1]) & (ly >= 0) & (ly < shape[0])
    if not inside[domain].all():
        raise ValueError('Audit crop does not contain the view-0 domain')
    own_v = np.zeros_like(domain)
    own_v[inside] = own[ly[inside], lx[inside]]
    accepted = domain & known
    counts = {
        'domain_view_pixels': int(domain.sum()),
        'accepted_view_pixels': int(accepted.sum()),
        'accepted_outside_own_mask': int((accepted & ~own_v).sum()),
        'own_mask_in_domain_not_accepted': int((domain & own_v & ~known).sum()),
        'neutral_outside_own_mask': int((domain & ~own_v & ~known).sum()),
    }
    # Own artwork not covered by any saved geometry: map every own-mask artwork
    # pixel centre forward into view 0 and test the geometry domain there.
    oy, ox = np.nonzero(own)
    fvx = np.floor((ox + x0 + 0.5 - cam_sx) * k + width / 2).astype(int)
    fvy = np.floor((oy + y0 + 0.5 - cam_sy) * k + height / 2).astype(int)
    in_frame = (fvx >= 0) & (fvx < width) & (fvy >= 0) & (fvy < height)
    hit = np.zeros(oy.shape, bool)
    hit[in_frame] = domain[fvy[in_frame], fvx[in_frame]]
    covered = np.zeros(shape, bool)
    covered[oy[hit], ox[hit]] = True
    counts['own_mask_source_pixels'] = int(own.sum())
    counts['own_mask_source_pixels_uncovered'] = int((own & ~covered).sum())
    # Masks are hand-traced envelopes (1-3 px uncertainty); count separately
    # the uncovered pixels farther than one pixel from covered geometry.
    near = covered.copy()
    for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)):
        near |= np.roll(np.roll(covered, dy, 0), dx, 1)
    counts['own_mask_source_pixels_uncovered_beyond_1px'] = int((own & ~near).sum())
    # RGB preservation of accepted pixels (nearest artwork sample; the view is
    # scaled by k, so edge texels can differ).
    diff = np.abs(textured[accepted] - source[sy[accepted], sx[accepted]]).sum(axis=1)
    counts['accepted_rgb_exact_fraction'] = round(float((diff == 0).mean()), 4) if diff.size else None
    counts['accepted_rgb_within_24_fraction'] = round(float((diff <= 24).mean()), 4) if diff.size else None
    # Name owners of neutral pixels outside our masks.
    neutral = domain & ~own_v & ~known
    owners = {}
    claimed = np.zeros_like(domain)
    for index, canvas in foreign_layers.items():
        hit = np.zeros_like(domain)
        hit[inside] = canvas[ly[inside], lx[inside]]
        n = int((neutral & hit & ~claimed).sum())
        claimed |= neutral & hit
        if n:
            owners[f'mask {index} ({",".join(sorted(foreign_nodes[index]))})'] = n
    owners['no native mask (ground/background/interior artwork)'] = int((neutral & ~claimed).sum())
    counts['neutral_owner_breakdown'] = dict(sorted(owners.items(), key=lambda kv: -kv[1]))
    rejected_own = domain & own_v & ~known
    if first_hit is not None:
        ys, xs = np.nonzero(rejected_own)
        names = first_hit(view, width, height, k, list(zip(xs.tolist(), ys.tolist())))
        owned_names = set(first_hit.owned)
        blockers = {}
        for name in names:
            key = ('own: ' + name) if name in owned_names else (name or 'no hit')
            blockers[key] = blockers.get(key, 0) + 1
        counts['own_mask_rejected_first_hit'] = dict(sorted(blockers.items(), key=lambda kv: -kv[1]))
        counts['own_mask_rejected_on_own_surface'] = sum(v for k, v in blockers.items() if k.startswith('own: ') or k == 'no hit')
    # Evidence image: artwork resampled into view 0 | textured | classification.
    art = np.zeros((height, width, 3), np.uint8)
    art[inside] = source[sy[inside], sx[inside]]
    cls = np.zeros((height, width, 3), np.uint8)
    cls[domain & known & own_v] = (40, 200, 60)        # accepted own artwork
    cls[domain & own_v & ~known] = (230, 40, 40)       # own artwork rejected
    cls[neutral & claimed] = (70, 110, 230)            # foreign-owned neutral
    cls[neutral & ~claimed] = (150, 150, 150)          # no mask: background/interior
    cls[accepted & ~own_v] = (255, 230, 0)             # accepted outside own mask
    unc = own & ~covered                                # own artwork missing geometry
    uy, ux = np.nonzero(unc)
    mvx = np.floor((ux + x0 + 0.5 - cam_sx) * k + width / 2).astype(int)
    mvy = np.floor((uy + y0 + 0.5 - cam_sy) * k + height / 2).astype(int)
    ok = (mvx >= 0) & (mvx < width) & (mvy >= 0) & (mvy < height)
    cls[mvy[ok], mvx[ok]] = (255, 0, 255)
    # Crop all panels to the domain (plus margin) and enlarge for inspection.
    dy, dx = np.nonzero(domain | (cls.sum(axis=2) > 0))
    cx0, cy0 = max(0, dx.min() - 12), max(0, dy.min() - 12)
    cx1, cy1 = min(width, dx.max() + 13), min(height, dy.max() + 13)
    zoom = max(1, min(6, 420 // max(cx1 - cx0, cy1 - cy0)))
    pw, ph = (cx1 - cx0) * zoom, (cy1 - cy0) * zoom
    sheet = Image.new('RGB', (max(pw * 3, 900), ph + 40), (0, 0, 0))
    for i, img in enumerate((art, textured.astype(np.uint8), cls)):
        panel = Image.fromarray(img[cy0:cy1, cx0:cx1]).resize((pw, ph), Image.NEAREST)
        sheet.paste(panel, (i * pw, 0))
    d = ImageDraw.Draw(sheet)
    d.text((4, ph + 4), 'artwork (source camera) | saved materials, view 0 | green accepted own, red own rejected,', fill=(255, 255, 255))
    d.text((4, ph + 20), 'blue foreign-mask neutral, grey unmasked neutral, magenta own artwork without geometry, yellow accepted outside own mask', fill=(255, 255, 255))
    out = workspace / 'inspection' / 'source-coverage.png'
    out.parent.mkdir(exist_ok=True)
    sheet.save(out)
    return {'own_mask_indices': own_indices, 'excluded_foreground_masks': sorted(excluded_indices),
            'reject_all_nodes': rejected_nodes,
            'counts': counts, 'evidence_image': str(out)}


if __name__ == '__main__':
    result = audit(sys.argv[1])
    print(json.dumps(result, indent=1))
