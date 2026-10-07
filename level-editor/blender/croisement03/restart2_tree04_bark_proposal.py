"""Trace three visible narrow bark tracks; reserve mixed ivy and canopy pixels."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement'
OUT = B / 'restart2/tree04-bark-proposal-v1'


def main():
    OUT.mkdir(exist_ok=False)
    src_path = B / 'baseline/covered.png'
    src = Image.open(src_path).convert('RGB')
    level = json.loads((B / 'baseline/Croisement03.rhp.json').read_text())
    domain = Image.new('L', src.size)
    domain.paste(Image.open(B / 'baseline/masks/000004.png'), tuple(level['masks'][4]['box_top_left']))
    polygons = [
        [(400,101),(405,104),(407,123),(411,139),(413,163),(407,166),(405,146),(402,132),(400,119)],
        [(433,92),(439,94),(439,116),(439,136),(439,158),(432,158),(432,137),(433,116)],
        [(458,84),(461,84),(461,106),(460,125),(460,147),(456,148),(456,128),(457,108)],
    ]
    proposed = Image.new('L', src.size)
    draw = ImageDraw.Draw(proposed)
    for polygon in polygons:
        draw.polygon(polygon, fill=255)
    rgb = np.array(src).astype(int)
    selected = (np.array(proposed) > 0) & (np.array(domain) > 0)
    selected &= (rgb[:, :, 0] >= rgb[:, :, 1]) & ((rgb[:, :, 1] - rgb[:, :, 2]) < 40)
    selected &= (rgb.max(axis=2) < 248) & (rgb.max(axis=2) > 25)
    selected &= (rgb.max(axis=2) - rgb.min(axis=2)) < 85
    neighbor = np.array(Image.open(B/'restart2/tree06-bark-proposal-v3/proposed-bark.png')) > 0
    selected &= ~neighbor
    assert not np.any(selected & neighbor)
    Image.fromarray(np.where(selected, 255, 0).astype('uint8')).save(OUT / 'proposed-bark.png')
    marked = np.array(src)
    marked[selected] = [255, 40, 170]
    box = (390, 75, 469, 174)
    sheet = Image.new('RGB', (750, 800))
    for i, image in enumerate((src, Image.fromarray(marked))):
        sheet.paste(image.crop(box).resize((375, 800), Image.Resampling.NEAREST), (i * 375, 0))
    sheet.save(OUT / 'proposal-comparison.png')
    (OUT / 'proposal.json').write_text(json.dumps(dict(status='PRIVATE positive bark proposal; self/root review pending',
        source_sha256=hashlib.sha256(src_path.read_bytes()).hexdigest(), mask=4,
        source_obstacles=[8,9,10], proposed_pixels=int(selected.sum()), polygons=polygons,
        morphology='Three positive woody tracks: broad curved left, pale central and thin right; correspond spatially to obstacle009,008,010. Thin dark tracks around425 and foliage-red upper forks remain unassigned. Ground transition and foreground foliage excluded.',
        limits=['Only visible muted bark cores proposed. All other foliage, ivy, dark recess and ground remain unassigned.',
                'Upper canopy and hidden roots require inference from own native context and only the two permitted Leicester references.',
                'Shared Arbre07 animation ownership and wind remain separate.']), indent=2) + '\n')


if __name__ == '__main__':
    main()
