"""Trace visible forked bark cores; reserve mixed ivy and canopy pixels."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement'
OUT = B / 'restart2/tree11-bark-proposal-v2'


def main():
    OUT.mkdir(exist_ok=False)
    src_path = B / 'baseline/covered.png'
    src = Image.open(src_path).convert('RGB')
    level = json.loads((B / 'baseline/Croisement03.rhp.json').read_text())
    domain = Image.new('L', src.size)
    domain.paste(Image.open(B / 'baseline/masks/000011.png'), tuple(level['masks'][11]['box_top_left']))
    polygons = [
        [(869, 61), (872, 63), (878, 85), (882, 99), (888, 121), (892, 139), (889, 141), (885, 124), (879, 103), (874, 86)],
        [(908, 88), (910, 89), (907, 106), (904, 120), (903, 142), (900, 145), (899, 132), (902, 115), (904, 104)],
    ]
    proposed = Image.new('L', src.size)
    draw = ImageDraw.Draw(proposed)
    for polygon in polygons:
        draw.polygon(polygon, fill=255)
    rgb = np.array(src).astype(int)
    selected = (np.array(proposed) > 0) & (np.array(domain) > 0)
    selected &= (rgb[:, :, 0] >= rgb[:, :, 1]) & ((rgb[:, :, 1] - rgb[:, :, 2]) < 40)
    selected &= (rgb.max(axis=2) < 215) & (rgb.max(axis=2) > 45)
    selected &= (rgb.max(axis=2) - rgb.min(axis=2)) < 65
    yy, xx = np.indices(selected.shape)
    selected &= (xx > 897) | (yy < 123)  # Reserve the deeply shadowed lower-left continuation.
    Image.fromarray(np.where(selected, 255, 0).astype('uint8')).save(OUT / 'proposed-bark.png')
    marked = np.array(src)
    marked[selected] = [255, 40, 170]
    box = (850, 20, 925, 180)
    sheet = Image.new('RGB', (750, 800))
    for i, image in enumerate((src, Image.fromarray(marked))):
        sheet.paste(image.crop(box).resize((375, 800), Image.Resampling.NEAREST), (i * 375, 0))
    sheet.save(OUT / 'proposal-comparison.png')
    (OUT / 'proposal.json').write_text(json.dumps(dict(status='PRIVATE positive bark proposal; self/root review pending',
        source_sha256=hashlib.sha256(src_path.read_bytes()).hexdigest(), mask=11,
        source_obstacles=[21, 22, 23], proposed_pixels=int(selected.sum()), polygons=polygons,
        morphology='One forked tree supported by a base volume and two rising limb volumes, not three independent tall stems.',
        limits=['Only visible muted bark cores proposed. All other foliage, ivy, dark recess and ground remain unassigned.',
                'Upper canopy and hidden lower fork require inference from own native context and only the two permitted Leicester references.',
                'Shared Arbre06 animation ownership and wind remain separate.']), indent=2) + '\n')


if __name__ == '__main__':
    main()
