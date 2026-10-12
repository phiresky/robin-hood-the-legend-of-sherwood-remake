"""Compose Lincoln per-state source artwork and change regions (system Python).

A state is a set of applied base patches. Its source is revealed.png (the Day
substrate) with every other patch's initial graphic and each applied patch's
applied graphic, in native patch order. Patches integrated into the background
(Pont_levis, Patch09, Patch05) bake their last transition frame beneath all
sprites. The empty state must reproduce covered.png exactly; that is checked.

The change region of a state is every pixel whose RGB differs from covered.png.
Outside it, covered-state ownership is unchanged by construction.

python3 level-editor/blender/lincoln/revealed_state_sources.py [--output DIR]
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'work/needs-review/lincoln-refinement'
SOURCE = WORK / 'source-states'

# Reviewed state selection. Each state lists the applied base patches it shows.
STATES = {
    'p000': ['patch-000'],
    'p000-p002': ['patch-000', 'patch-002'],
    'p001': ['patch-001'],
    'p003': ['patch-003'],
    'p005': ['patch-005'],
    'p005-p004': ['patch-005', 'patch-004'],
    'p006': ['patch-006'],
    'p007': ['patch-007'],
    'p008': ['patch-008'],
    'p010': ['patch-010'],
    'p011': ['patch-011'],
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


# Initial graphics nested inside another patch's reveal cover (interior art), reviewed.
# The approved v1 state sources keep them (they match covered.png v1); the v2 covered
# source (covered_source_v2.py) omits them while their cover is unapplied.
REVIEWED_NESTED = {('patch-002', 'patch-000')}


def alpha_mask(layers, patch_id, key, shape):
    record = next(r for r in layers['patches'] if r['id'] == patch_id)
    graphic = record[key]
    mask = np.zeros(shape, dtype=bool)
    if graphic is None:
        return mask
    alpha = np.asarray(Image.open(SOURCE / graphic['image']).convert('RGBA'))[:, :, 3] > 0
    x, y, w, h = graphic['bbox']
    x0, y0, x1, y1 = max(x, 0), max(y, 0), min(x + w, shape[1]), min(y + h, shape[0])
    mask[y0:y1, x0:x1] = alpha[y0 - y:y1 - y, x0 - x:x1 - x]
    return mask


def nested_initial_graphics(layers, shape=(2176, 2944)):
    """Initial graphics fully inside an earlier patch's initial cover graphic."""
    masks = {r['id']: alpha_mask(layers, r['id'], 'initial_graphic', shape) for r in layers['patches']}
    order = [r['id'] for r in layers['patches']]
    result = []
    for later in order:
        inner = masks[later]
        if not inner.any():
            continue
        for cover in order[:order.index(later)]:
            if masks[cover].any() and not (inner & ~masks[cover]).any():
                result.append({'patch': later, 'cover': cover, 'pixels': int(inner.sum()),
                               'name': next(r['name'] for r in layers['patches'] if r['id'] == later)})
    return result


def check_nested(layers):
    """Guard: any interior sprite composited over an unapplied cover must be reviewed."""
    found = {(row['patch'], row['cover']) for row in nested_initial_graphics(layers)}
    if found != REVIEWED_NESTED:
        raise ValueError(f'Unreviewed nested interior graphics: {sorted(found ^ REVIEWED_NESTED)}')
    return found


def compose(layers, applied, omit_nested=False):
    canvas = Image.open(SOURCE / layers['sources']['interior']).convert('RGBA')
    ordered = sorted(layers['patches'], key=lambda r: not (
        r['id'] in applied and r['state'].get('integrate_in_background')))
    omitted = {inner for inner, cover in check_nested(layers) if cover not in applied} if omit_nested else set()
    for record in ordered:
        graphic = record['applied_graphic'] if record['id'] in applied else record['initial_graphic']
        if graphic is None or (record['id'] in omitted and record['id'] not in applied):
            continue
        sprite = Image.open(SOURCE / graphic['image']).convert('RGBA')
        canvas.alpha_composite(sprite, tuple(graphic['bbox'][:2]))
    return canvas.convert('RGB')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=WORK / 'state-review/sources')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    layers = json.loads((SOURCE / 'layers.json').read_text())
    covered = np.asarray(Image.open(SOURCE / 'covered.png').convert('RGB'))
    check_nested(layers)
    check = np.asarray(compose(layers, set()))
    mismatch = int(np.count_nonzero(np.any(check != covered, axis=2)))
    if mismatch:
        raise ValueError(f'Covered recomposition differs from covered.png: {mismatch} pixels')
    records = {}
    for state, patches in STATES.items():
        known = {r['id'] for r in layers['patches']}
        if set(patches) - known:
            raise ValueError('Unknown patch in state ' + state)
        image = compose(layers, set(patches))
        path = out / f'{state}.png'
        if path.exists():
            if np.any(np.asarray(Image.open(path).convert('RGB')) != np.asarray(image)):
                raise FileExistsError(f'Existing state source differs: {path}')
        else:
            image.save(path)
        change = np.any(np.asarray(image) != covered, axis=2)
        region = out / f'{state}-change.png'
        Image.fromarray((change * 255).astype(np.uint8)).save(region)
        ys, xs = np.nonzero(change)
        records[state] = {
            'applied_patches': patches, 'image': str(path), 'sha256': sha(path),
            'change_region': str(region), 'change_region_sha256': sha(region),
            'changed_pixels': int(change.sum()),
            'change_bbox': [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1],
        }
    manifest = {'version': 1, 'map': 'Lincoln', 'covered': str(SOURCE / 'covered.png'),
                'covered_sha256': sha(SOURCE / 'covered.png'),
                'covered_recomposition_mismatch': mismatch,
                'rule': 'Day substrate + initial graphics of unapplied patches + applied graphics of applied '
                        'patches in native order; integrate_in_background applied frames first.',
                'states': records}
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({k: v['changed_pixels'] for k, v in records.items()}))


if __name__ == '__main__':
    main()
