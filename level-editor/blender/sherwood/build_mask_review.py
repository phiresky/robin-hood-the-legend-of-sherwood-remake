"""Render explicit candidate associations for inspection; never infer or approve them."""
import hashlib
import html
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

EDITOR = Path(__file__).resolve().parents[2]
RECIPE = Path(__file__).with_name('source-mask-candidates.json')
ROOT = EDITOR/'work/needs-review/sherwood-refinement/textures/native-mask-audit/assignments'
INVENTORY = EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Sherwood.rhp.d/masks/manifest.json'
SOURCE = EDITOR.parent/'datadirs/fullgame_gog_hackable/Data/Levels/Day/sherwood.map.png'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bitmap(record, size):
    canvas = Image.new('L', size)
    patch = Image.open(INVENTORY.parent/record['png']).convert('L')
    assert patch.size == tuple(record['box_size'])
    canvas.paste(patch, tuple(record['box_top_left']))
    return np.asarray(canvas) > 0


def main():
    ROOT.mkdir(parents=True, exist_ok=True)
    recipe = json.loads(RECIPE.read_text())
    original = Image.open(SOURCE).convert('RGB')
    records = {r['index']: r for r in json.loads(INVENTORY.read_text())['masks']}
    masks = {i: bitmap(r, original.size) for i, r in records.items()}
    font = ImageFont.load_default(size=17)
    cards = []
    manifest = []
    for rule in recipe['assignments']:
        allowed = np.logical_or.reduce([masks[i] for i in rule['mask_indices']])
        union = allowed.copy()
        for i in rule.get('exclude_mask_indices', []):
            allowed &= ~masks[i]
        ys, xs = np.where(union)
        box = (max(0, int(xs.min())-8), max(0, int(ys.min())-8),
               min(original.width, int(xs.max())+9), min(original.height, int(ys.max())+9))
        cutout = Image.new('RGB', original.size, '#17202b')
        cutout.paste(original, (0, 0), Image.fromarray(allowed.astype('uint8')*255))
        card = Image.new('RGB', (720, 460), '#272e38')
        draw = ImageDraw.Draw(card)
        unresolved = rule['source_node'] in recipe['unresolved']
        draw.text((8, 5), f"{rule['source_node']}  +{rule['mask_indices']} -{rule.get('exclude_mask_indices', [])}", font=font, fill='white')
        draw.text((8, 29), 'UNRESOLVED' if unresolved else 'Candidate association', font=font, fill='#ffcc66')
        for col, picture in enumerate([original, cutout]):
            crop = picture.crop(box)
            scale = min(350/crop.width, 390/crop.height, 4)
            crop = crop.resize((round(crop.width*scale), round(crop.height*scale)), Image.Resampling.NEAREST)
            card.paste(crop, (col*360+(360-crop.width)//2, 65+(390-crop.height)//2))
        path = ROOT/(rule['source_node']+'.png')
        card.save(path)
        cards.append(card)
        manifest.append(dict(source_node=rule['source_node'], file=path.name, sha256=digest(path),
                             accepted_pixels=int(allowed.sum()), native_union_pixels=int(union.sum()), unresolved=unresolved))
    for start in range(0, len(cards), 6):
        page = Image.new('RGB', (1440, 1380))
        for offset, card in enumerate(cards[start:start+6]):
            page.paste(card, ((offset%2)*720, (offset//2)*460))
        page.save(ROOT/f'page-{start//6:02}.png')
    (ROOT/'manifest.json').write_text(json.dumps(dict(status='CANDIDATE_NOT_APPROVED', recipe_sha256=digest(RECIPE),
        inventory_sha256=digest(INVENTORY), source_sha256=digest(SOURCE), cards=manifest), indent=2)+'\n')
    content = '<!doctype html><meta charset="utf-8"><title>Sherwood source ownership audit</title><style>body{background:#18202a;color:white;font:16px system-ui;margin:24px}img{max-width:100%}article{display:inline-block;vertical-align:top;width:720px;margin:6px}p{max-width:1000px}</style>'
    content += '<h1>Sherwood source ownership audit</h1><p>Candidate masks, not approved texture revisions. Original Day art is on the left; eligible pixels after native-mask exclusions are on the right. Unresolved assignments must stay unknown. These images do not replace model or grouping reviews.</p>'
    for row in manifest:
        content += '<article><img loading="lazy" src="'+row['file']+'"><p>'+html.escape(recipe['unresolved'].get(row['source_node'], 'Assignment overlay awaiting inspection.'))+'</p></article>'
    (ROOT/'index.html').write_text(content)
    print(f'Rendered {len(cards)} explicit association cards; {len(recipe["unresolved"])} unresolved source nodes.')


if __name__ == '__main__':
    main()
