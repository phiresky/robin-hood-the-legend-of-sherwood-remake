"""Freeze source-states-v2/covered.png: covered artwork without nested interior sprites.

python3 level-editor/blender/lincoln/covered_source_v2.py

covered.png (v1) composites every base patch's initial graphic in native order.
An initial graphic that lies inside another patch's still-unapplied reveal cover is
interior art: in game it is only visible once that cover has been removed. v2 omits
such a graphic wherever the cover patch is unapplied; the only Lincoln case is the
patch08 closed door (patch-002) inside the Patch02 terrace roof (patch-000).
Pixels outside the omitted sprite are byte-identical to v1 (checked). v1 stays frozen.
"""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

import revealed_state_sources as RSS

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'work/needs-review/lincoln-refinement'
SOURCE = WORK / 'source-states'
OUT = WORK / 'source-states-v2'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    layers = json.loads((SOURCE / 'layers.json').read_text())
    nested = RSS.nested_initial_graphics(layers)
    if [(n['patch'], n['cover']) for n in nested] != [('patch-002', 'patch-000')]:
        raise ValueError(f'Unexpected nested interior graphics: {nested}')
    v1 = np.asarray(Image.open(SOURCE / 'covered.png').convert('RGB'))
    v2 = np.asarray(RSS.compose(layers, set(), omit_nested=True))
    sprite = RSS.alpha_mask(layers, 'patch-002', 'initial_graphic', v1.shape[:2])
    diff = np.any(v1 != v2, axis=2)
    if (diff & ~sprite).any():
        raise ValueError('v2 differs from v1 outside the omitted sprite')
    OUT.mkdir(exist_ok=True)
    covered, mask = OUT / 'covered.png', OUT / 'covered-v1-diff.png'
    for path, data in ((covered, v2), (mask, (diff * 255).astype(np.uint8))):
        image = Image.fromarray(data)
        if path.exists():
            if not np.array_equal(np.asarray(Image.open(path)), data):
                raise FileExistsError(f'Existing {path} differs; choose a new version')
        else:
            image.save(path)
    ys, xs = np.nonzero(diff)
    manifest = {
        'version': 2, 'map': 'Lincoln',
        'rule': ('Day substrate plus every base patch initial graphic in native order, except an initial '
                 'graphic lying inside another patch\'s unapplied reveal cover (interior art).'),
        'omitted': [{'patch': n['patch'], 'name': n['name'], 'inside_cover': n['cover'],
                     'sprite_pixels': n['pixels']} for n in nested],
        'covered': str(covered), 'covered_sha256': sha(covered),
        'v1': str(SOURCE / 'covered.png'), 'v1_sha256': sha(SOURCE / 'covered.png'),
        'v1_diff_mask': str(mask), 'v1_diff_mask_sha256': sha(mask),
        'changed_pixels': int(diff.sum()),
        'changed_bbox': [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1],
        'outside_sprite_changes': 0,
        'user_review': 'scratch/states/vizout/door-fix.png: "looks good"',
    }
    (OUT / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({k: manifest[k] for k in ('covered_sha256', 'changed_pixels', 'changed_bbox')}))


if __name__ == '__main__':
    main()
