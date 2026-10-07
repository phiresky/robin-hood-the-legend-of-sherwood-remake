"""Trace three visible narrow bark tracks; reserve mixed ivy and canopy pixels."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement'
OUT = B / 'restart2/tree03-bark-proposal-v1'


def main():
    OUT.mkdir(exist_ok=False)
    src_path = B / 'baseline/covered.png'
    src = Image.open(src_path).convert('RGB')
    level = json.loads((B / 'baseline/Croisement03.rhp.json').read_text())
    domain = Image.new('L', src.size)
    domain.paste(Image.open(B / 'baseline/masks/000003.png'), tuple(level['masks'][3]['box_top_left']))
    polygons = [
        [(286,119),(292,116),(299,125),(301,142),(299,163),(285,163),(288,140)],
        [(295,64),(299,66),(304,94),(304,110),(299,115),(296,102),(295,84)],
        [(298,114),(316,106),(334,99),(351,91),(356,86),(357,93),(338,107),(318,116),(304,124)],
        [(291,117),(276,111),(270,109),(266,113),(277,120),(291,128)],
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
    box = (255, 55, 365, 174)
    sheet = Image.new('RGB', (750, 800))
    for i, image in enumerate((src, Image.fromarray(marked))):
        sheet.paste(image.crop(box).resize((375, 800), Image.Resampling.NEAREST), (i * 375, 0))
    sheet.save(OUT / 'proposal-comparison.png')
    (OUT / 'proposal.json').write_text(json.dumps(dict(status='PRIVATE positive bark proposal; self/root review pending',
        source_sha256=hashlib.sha256(src_path.read_bytes()).hexdigest(), mask=3,
        source_obstacles=[7], proposed_pixels=int(selected.sum()), polygons=polygons,
        morphology='One broad mature trunk with upright fork, long right branch and short left branch; mapped to obstacle007 as provenance. Far-left branch and neighboring Tree02 stem remain excluded. Red leaves, bright leaf patches, unclassified forks and ground contact remain unassigned.',
        limits=['Only visible muted bark cores proposed. All other foliage, ivy, dark recess and ground remain unassigned.',
                'Upper canopy and hidden roots require inference from own native context and only the two permitted Leicester references.',
                'Shared Arbre08 animation ownership and wind remain separate.']), indent=2) + '\n')


if __name__ == '__main__':
    main()
