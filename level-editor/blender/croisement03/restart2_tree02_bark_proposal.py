"""Trace three visible narrow bark tracks; reserve mixed ivy and canopy pixels."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement'
OUT = B / 'restart2/tree02-bark-proposal-v1'


def main():
    OUT.mkdir(exist_ok=False)
    src_path = B / 'baseline/covered.png'
    src = Image.open(src_path).convert('RGB')
    level = json.loads((B / 'baseline/Croisement03.rhp.json').read_text())
    domain = Image.new('L', src.size)
    domain.paste(Image.open(B / 'baseline/masks/000002.png'), tuple(level['masks'][2]['box_top_left']))
    polygons = [
        [(207,135),(211,136),(212,148),(214,166),(209,166),(207,150)],
        [(199,145),(202,145),(203,159),(201,162),(198,159)],
        [(195,142),(203,138),(209,131),(211,132),(205,140),(198,145)],
        [(208,120),(213,116),(215,119),(211,127),(211,136),(207,135)],
        [(212,122),(219,121),(229,126),(239,120),(249,110),(251,114),(243,125),(232,132),(222,130),(211,127)],
    ]
    proposed = Image.new('L', src.size)
    draw = ImageDraw.Draw(proposed)
    for polygon in polygons:
        draw.polygon(polygon, fill=255)
    rgb = np.array(src).astype(int)
    selected = np.array(proposed) > 0
    selected &= (rgb[:, :, 0] >= rgb[:, :, 1]) & ((rgb[:, :, 1] - rgb[:, :, 2]) < 40)
    selected &= (rgb.max(axis=2) < 248) & (rgb.max(axis=2) > 25)
    selected &= (rgb.max(axis=2) - rgb.min(axis=2)) < 85
    neighbor = np.array(Image.open(B/'restart2/tree03-bark-proposal-v1/proposed-bark.png')) > 0
    selected &= ~neighbor
    assert not np.any(selected & neighbor)
    Image.fromarray(np.where(selected, 255, 0).astype('uint8')).save(OUT / 'proposed-bark.png')
    marked = np.array(src)
    marked[selected] = [255, 40, 170]
    box = (188, 108, 256, 174)
    sheet = Image.new('RGB', (750, 800))
    for i, image in enumerate((src, Image.fromarray(marked))):
        sheet.paste(image.crop(box).resize((375, 800), Image.Resampling.NEAREST), (i * 375, 0))
    sheet.save(OUT / 'proposal-comparison.png')
    (OUT / 'proposal.json').write_text(json.dumps(dict(status='PRIVATE positive bark proposal; self/root review pending',
        source_sha256=hashlib.sha256(src_path.read_bytes()).hexdigest(), mask=2,
        source_obstacles=[6], proposed_pixels=int(selected.sum()), proposed_outside_native_mask_pixels=int((selected & ~(np.array(domain)>0)).sum()), polygons=polygons,
        morphology='Thin Tree02 stem/root tracks and attached rightward bent branch, mapped to obstacle006. Right branch extends outside narrow native mask2 into a region explicitly excluded from Tree03 positive bark; it is proposed as an authored source extension requiring independent review. Red foliage, far-left branches and ground remain unassigned.',
        limits=['Only visible muted bark cores proposed. All other foliage, ivy, dark recess and ground remain unassigned.',
                'Upper canopy and hidden roots require inference from own native context and only the two permitted Leicester references.',
                'Shared Arbre08 animation ownership and wind remain separate.']), indent=2) + '\n')


if __name__ == '__main__':
    main()
