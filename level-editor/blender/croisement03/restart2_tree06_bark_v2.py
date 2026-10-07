"""Trace three visible narrow bark tracks; reserve mixed ivy and canopy pixels."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement'
OUT = B / 'restart2/tree06-bark-proposal-v2'


def main():
    OUT.mkdir(exist_ok=False)
    src_path = B / 'baseline/covered.png'
    src = Image.open(src_path).convert('RGB')
    level = json.loads((B / 'baseline/Croisement03.rhp.json').read_text())
    domain = Image.new('L', src.size)
    domain.paste(Image.open(B / 'baseline/masks/000006.png'), tuple(level['masks'][6]['box_top_left']))
    polygons = [
        [(563,72),(567,70),(568,100),(567,128),(567,151),(566,157),(562,156),(562,149),(562,122)],
        [(578,66),(584,63),(585,95),(582,113),(583,144),(583,166),(578,171),(572,169),(571,151),(574,129),(573,102),(575,82)],
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
    Image.fromarray(np.where(selected, 255, 0).astype('uint8')).save(OUT / 'proposed-bark.png')
    marked = np.array(src)
    marked[selected] = [255, 40, 170]
    box = (545, 50, 610, 205)
    sheet = Image.new('RGB', (750, 800))
    for i, image in enumerate((src, Image.fromarray(marked))):
        sheet.paste(image.crop(box).resize((375, 800), Image.Resampling.NEAREST), (i * 375, 0))
    sheet.save(OUT / 'proposal-comparison.png')
    (OUT / 'proposal.json').write_text(json.dumps(dict(status='PRIVATE positive bark proposal; self/root review pending',
        source_sha256=hashlib.sha256(src_path.read_bytes()).hexdigest(), mask=6,
        source_obstacles=[13], proposed_pixels=int(selected.sum()), polygons=polygons,
        morphology='Two observed woody tracks, dark slender left and ivy-clad broader right. Their hidden connection under rock/ivy is inferred; one coarse obstacle does not establish biological trunk count.',
        limits=['Only visible muted bark cores proposed. All other foliage, ivy, dark recess and ground remain unassigned.',
                'Upper canopy and hidden roots require inference from own native context and only the two permitted Leicester references.',
                'Shared Arbre06 animation ownership and wind remain separate.']), indent=2) + '\n')


if __name__ == '__main__':
    main()
