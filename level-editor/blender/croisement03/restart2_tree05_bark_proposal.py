"""Trace three visible narrow bark tracks; reserve mixed ivy and canopy pixels."""
import hashlib
import json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement'
OUT = B / 'restart2/tree05-bark-proposal-v1'


def main():
    OUT.mkdir(exist_ok=False)
    src_path = B / 'baseline/covered.png'
    src = Image.open(src_path).convert('RGB')
    level = json.loads((B / 'baseline/Croisement03.rhp.json').read_text())
    domain = Image.new('L', src.size)
    domain.paste(Image.open(B / 'baseline/masks/000005.png'), tuple(level['masks'][5]['box_top_left']))
    polygons = [
        [(497,75),(501,74),(502,98),(501,116),(499,132),(495,130),(496,110)],
        [(515,52),(519,55),(518,80),(516,107),(514,135),(509,135),(509,113),(510,91),(512,72)],
        [(534,89),(537,88),(536,115),(534,137),(534,150),(530,149),(531,133),(532,111)],
        [(552,73),(557,73),(557,104),(557,123),(554,149),(550,149),(551,132),(551,108)],
        [(563,72),(567,70),(568,100),(567,128),(567,151),(566,157),(562,156),(562,149),(562,122)],
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
    box = (480, 50, 575, 180)
    sheet = Image.new('RGB', (750, 800))
    for i, image in enumerate((src, Image.fromarray(marked))):
        sheet.paste(image.crop(box).resize((375, 800), Image.Resampling.NEAREST), (i * 375, 0))
    sheet.save(OUT / 'proposal-comparison.png')
    (OUT / 'proposal.json').write_text(json.dumps(dict(status='PRIVATE positive bark proposal; self/root review pending',
        source_sha256=hashlib.sha256(src_path.read_bytes()).hexdigest(), mask=5,
        source_obstacles=[11,12,14], proposed_pixels=int(selected.sum()), polygons=polygons,
        morphology='Five visible woody tracks in the woodland cluster. Three native obstacle parts are provenance only, not proof of five biological trees; central thin track may require authored scenery ownership. Roots are hidden by ivy and the rock shelf. Tree06 broader right stem is excluded.',
        limits=['Only visible muted bark cores proposed. All other foliage, ivy, dark recess and ground remain unassigned.',
                'Upper canopy and hidden roots require inference from own native context and only the two permitted Leicester references.',
                'Shared Arbre06 animation ownership and wind remain separate.']), indent=2) + '\n')


if __name__ == '__main__':
    main()
