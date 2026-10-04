"""Build source-artwork contact sheets for explicit Croisement03 asset ownership review."""
import json
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement03-refinement'


def main():
    level = json.loads((OUT / 'baseline/Croisement03.rhp.json').read_text())
    source = Image.open(OUT / 'baseline/covered.png').convert('RGB')
    output = OUT / 'survey'
    output.mkdir(exist_ok=True)
    font = ImageFont.load_default(size=15)
    records = []
    paths = sorted((ROOT / 'level-editor/library/3d-assets/croisement03').glob('*/asset.json'))
    cards = []
    for path in paths:
        descriptor = json.loads(path.read_text())
        ids = [p['source_obstacle'] for p in descriptor['parts'] if 'source_obstacle' in p]
        if not ids:
            continue
        points = [p for i in ids for p in level['sight_obstacles'][i]['points']]
        xs = [p['x'] for p in points]
        ys = [p['y'] - p[z] for p in points for z in ['z_bottom', 'z_top']]
        box = [max(0, math.floor(min(xs))-24), max(0, math.floor(min(ys))-24),
               min(source.width, math.ceil(max(xs))+24), min(source.height, math.ceil(max(ys))+24)]
        crop = source.crop(box).convert('RGBA')
        overlay = Image.new('RGBA', crop.size)
        draw = ImageDraw.Draw(overlay)
        for i in ids:
            pts = level['sight_obstacles'][i]['points']
            roof = [(p['x']-box[0], p['y']-p['z_top']-box[1]) for p in pts]
            draw.polygon(roof, fill=(20,220,255,45), outline=(30,255,255,220), width=2)
        crop = Image.alpha_composite(crop, overlay).convert('RGB')
        key = descriptor['id'].removeprefix('croisement03-')
        crop.save(output / f'{key}.jpg', quality=92)
        card = Image.new('RGB', (270,250), '#242424')
        crop.thumbnail((268,207))
        card.paste(crop, ((270-crop.width)//2, 22+(207-crop.height)//2))
        d = ImageDraw.Draw(card)
        d.text((5,2),key,fill='white',font=font)
        label = ','.join(map(str,ids))
        d.text((5,230),label[:34]+('…' if len(label)>34 else ''),fill='white',font=font)
        cards.append(card)
        records.append({'id': descriptor['id'], 'obstacles': ids, 'bounds_source_pixels': box,
                        'crop': f'{key}.jpg'})
    for start in range(0,len(cards),24):
        sheet = Image.new('RGB',(1620,1000),'#242424')
        for n,c in enumerate(cards[start:start+24]):sheet.paste(c,((n%6)*270,(n//6)*250))
        sheet.save(output / f'sheet-{start//24:02}.jpg',quality=93)
    (output/'index.json').write_text(json.dumps(records,indent=2)+'\n')
    print(json.dumps({'cards':len(cards),'sheets':math.ceil(len(cards)/24)}))


if __name__ == '__main__':
    main()
