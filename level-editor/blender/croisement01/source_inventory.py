"""Inventory native evidence and render mask context without assigning ownership."""
import hashlib
import json
from collections import Counter
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement01-refinement'
DATA = ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    target = OUT / 'source-survey'
    target.mkdir(parents=True, exist_ok=True)
    source = DATA / 'Day/Croisement01.map.png'
    level_path = DATA / 'Croisement01.rhp.json'
    level = json.loads(level_path.read_text())
    manifest = DATA / 'Croisement01.rhp.d/masks/manifest.json'
    masks = json.loads(manifest.read_text())['masks']
    artwork = Image.open(source).convert('RGB')
    cards, records = [], []
    for row in masks:
        index = row['index']
        x, y = row['box_top_left']
        width, height = row['box_size']
        path = manifest.parent / row['png']
        alpha = Image.open(path).convert('L')
        assert alpha.size == (width, height)
        rgb = artwork.crop((x, y, x + width, y + height))
        cutout = Image.new('RGB', rgb.size, '#333333')
        cutout.paste(rgb, mask=alpha)
        cutout.save(target / f'mask-{index:03}.png')
        context_box = (max(0, x-24), max(0, y-24), min(artwork.width, x+width+24), min(artwork.height, y+height+24))
        context = artwork.crop(context_box)
        context.save(target / f'context-{index:03}.png')
        card = Image.new('RGB', (320, 300), '#222222')
        cutout.thumbnail((155, 265))
        context.thumbnail((155, 265))
        card.paste(cutout, (2+(155-cutout.width)//2, 28+(265-cutout.height)//2))
        card.paste(context, (162+(155-context.width)//2, 28+(265-context.height)//2))
        count = int(np.count_nonzero(np.asarray(alpha)))
        ImageDraw.Draw(card).text((5, 5), f"Mask {index:03} | layer {row['layer']} | type {row['mask_type']} | {count}px", fill='white')
        cards.append(card)
        records.append({**{key: row[key] for key in ('index','layer','layer_index','mask_type','box_top_left','box_size')},
                        'nonzero_pixels': count, 'mask_sha256': sha(path),
                        'context': f'context-{index:03}.png', 'cutout': f'mask-{index:03}.png',
                        'ownership': 'unreviewed'})
    for start in range(0, len(cards), 12):
        sheet = Image.new('RGB', (1280, 900), '#222222')
        for offset, card in enumerate(cards[start:start+12]):
            sheet.paste(card, (offset % 4 * 320, offset // 4 * 300))
        sheet.save(target / f'masks-{start:03}-{min(start+11,len(cards)-1):03}.jpg', quality=94)
    report = dict(map='Croisement01', source_sha256=sha(source), level_sha256=sha(level_path),
                  size=artwork.size, masks=records,
                  counts={key: len(level[key]) for key in ('masks','sight_obstacles','animations','patches')},
                  layers=dict(Counter(row['layer'] for row in masks)),
                  animations=[dict(index=i, **row) for i,row in enumerate(level['animations'])],
                  status='Inventory only; source masks are not yet asset ownership.',
                  depth_review=dict(layers=[0,1], reviewed=True,
                                    notes='Layer 0 shows forest trunks, rocks, stumps and undergrowth; layer 1 contains the separate masks 41 and 63. Character threshold values are not physical depth.'))
    (target / 'inventory.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report['counts']))


if __name__ == '__main__':
    main()
