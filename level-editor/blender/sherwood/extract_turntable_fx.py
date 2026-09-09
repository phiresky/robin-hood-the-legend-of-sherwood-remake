"""Pack all ambient animation frames on fixed canvases for the turntable."""
import json
import math
from pathlib import Path
import numpy as np
from PIL import Image
from paths import DATA

OUT = Path(__file__).resolve().parents[3] / 'level-editor/work/sherwood-refinement/animation-references'
records = json.loads((OUT / 'manifest.json').read_text())['assets']
for r in records:
    if r['kind'] != 'ambient':
        continue
    images = []
    for path in r['source_frames']:
        parts = Path(path).parts
        source = DATA.joinpath(*parts[parts.index('Animations'):])
        a = np.array(Image.open(source).convert('RGBA'))
        a[np.all(a[:, :, :3] == [0, 251, 0], axis=2)] = 0
        images.append(Image.fromarray(a))
    minx = math.floor(min(p[0] for p in r['offsets']))
    miny = math.floor(min(p[1] for p in r['offsets']))
    w = math.ceil(max(p[0] + im.width for p, im in zip(r['offsets'], images))) - minx
    h = math.ceil(max(p[1] + im.height for p, im in zip(r['offsets'], images))) - miny
    cols = math.ceil(math.sqrt(len(images)))
    rows = math.ceil(len(images) / cols)
    atlas = Image.new('RGBA', (cols * (w + 4), rows * (h + 4)))
    for i, (im, offset) in enumerate(zip(images, r['offsets'])):
        atlas.alpha_composite(im, ((i % cols) * (w + 4) + 2 + round(offset[0]) - minx,
                                  (i // cols) * (h + 4) + 2 + round(offset[1]) - miny))
    r.update(turntable_atlas=f'ambient-{r["index"]:02}-atlas.png', canvas=[w, h],
             canvas_offset=[minx, miny], atlas_size=list(atlas.size), columns=cols, rows=rows)
    atlas.save(OUT / r['turntable_atlas'])
(OUT / 'turntable-fx.json').write_text(json.dumps([r for r in records if r['kind'] == 'ambient'], indent=2))
print('Packed all frames for 14 ambient overlays')
