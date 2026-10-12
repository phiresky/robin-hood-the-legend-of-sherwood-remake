"""Create a portable HTML review and a compact nine-map contact sheet."""
import base64
import html
import json
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / 'work/needs-review/wall-presets'


def main():
    rows = json.loads((WORK / 'segments.json').read_text())
    cards = []
    representatives = {}
    font = ImageFont.load_default(size=18)
    for row in rows:
        image = WORK / 'segments' / (row['id'] + '.webp')
        data = json.loads(image.with_suffix('.json').read_text())
        if not data.get('model_sha256'):
            raise ValueError('Final full-model audit missing: ' + row['id'])
        uri = 'data:image/webp;base64,' + base64.b64encode(image.read_bytes()).decode()
        escape = html.escape
        descriptor = json.loads((WORK / 'staging/3d-assets' / row['id'] / 'asset.json').read_text())
        repeat_check = descriptor['provenance'].get('repeat_check')
        spacing = ''
        if repeat_check:
            gaps = repeat_check['internal_gaps']
            spacing = (f"<br>{repeat_check['features']} complete features per repeat; "
                       f"join gap {repeat_check['seam_gap']:.2f}")
            if gaps:spacing += f"; interior gaps {min(gaps):.2f}–{max(gaps):.2f}"
        cards.append(f'''<article data-map="{escape(row['source_map'])}">
          <h2>{escape(row['name'])}</h2><p>Source: <code>{escape(row['source'])}</code><br>
          Width {row['width']:.1f} · Repeat {row['repeatLength']:.1f} · Height {row['height']:.1f}{spacing}</p>
          <details><summary>Original, strip, repeats, curve, corner, and join close-up — both cameras</summary>
          <img loading="lazy" src="{uri}" alt="{escape(row['name'])} comparison"></details></article>''')
        representatives.setdefault(row['source_map'], image)
    maps = sorted(representatives)
    options = ''.join(f'<option>{html.escape(name)}</option>' for name in maps)
    document = f'''<!doctype html><meta charset="utf-8"><title>Wall preset visual review</title>
    <style>body{{margin:32px auto;max-width:1100px;padding:0 20px;font:16px system-ui;background:#171e24;color:#e5e9ec}}
    p{{line-height:1.55;color:#c0cad1}}article{{padding:18px 0;border-bottom:1px solid #485660}}h2{{font-size:20px}}
    img{{width:100%;height:auto}}summary,select{{cursor:pointer;padding:12px;background:#2b3740;color:inherit;border:0}}
    code{{overflow-wrap:anywhere}}[hidden]{{display:none}}</style>
    <h1>Wall, fence, bank and railing presets</h1>
    <p>{len(rows)} prepared types from {len(maps)} maps. Wychford is excluded. Each comparison uses the full models,
    in the game camera and an angled view. Rows show the original source asset, the dedicated strip,
    three repeats, an S-curve, a corner, and an enlarged repeat join. Source UVs/materials are retained; strip straightening
    and leveling are confined to copies. Original shared assets and map placements are unchanged.</p>
    <p>These are segment and spline comparisons, not reconstructions of every wall in the original levels.
    Projection-only woodland/York assets retain their source texture limitations; repeating shadows
    and texture seams are visible in some types.</p>
    <label>Source map <select id="filter"><option value="">All maps</option>{options}</select></label>
    {''.join(cards)}<script>document.querySelector('#filter').onchange=e=>document.querySelectorAll('article').forEach(a=>a.hidden=!!e.target.value&&a.dataset.map!==e.target.value)</script>'''
    (WORK / 'review.html').write_text(document)
    sheet = Image.new('RGB', (1500, 1320), '#20272e')
    draw = ImageDraw.Draw(sheet)
    for index, name in enumerate(maps):
        x, y = index % 3 * 500, index // 3 * 440
        image = Image.open(representatives[name])
        # The two-view curved run, followed by its corner test.
        crop = image.crop((0, 1020, 1000, 1700)).resize((500, 340))
        sheet.paste(crop, (x, y + 48))
        draw.text((x + 12, y + 16), name, font=font, fill='#ecf0e5')
    sheet.save(WORK / 'overview.jpg', quality=92)
    strips = Image.new('RGB', (1500, ((len(rows) + 2) // 3) * 210), '#20272e')
    labels = ImageDraw.Draw(strips)
    for index, row in enumerate(rows):
        x, y = index % 3 * 500, index // 3 * 210
        image = Image.open(WORK / 'segments' / (row['id'] + '.webp'))
        strips.paste(image.crop((0, 1020, 1000, 1360)).resize((500, 170)), (x, y + 35))
        labels.text((x + 8, y + 7), row['name'], font=font, fill='#ecf0e5')
    strips.save(WORK / 'all-strips.jpg', quality=94)
    joins = Image.new('RGB', (1000, len(rows) * 375), '#20272e')
    labels = ImageDraw.Draw(joins)
    for index,row in enumerate(rows):
        image = Image.open(WORK / 'segments' / (row['id'] + '.webp'))
        if image.height < 2040:raise ValueError('Join close-up missing: '+row['id'])
        crop = image.crop((0,1700,1000,2040))
        crop.save(WORK / 'segments' / (row['id'] + '-join.png'))
        image.crop((0,1360,1000,1700)).save(WORK / 'segments' / (row['id'] + '-corner.png'))
        joins.paste(crop,(0,index*375+35))
        labels.text((12,index*375+8),row['name'],font=font,fill='#ecf0e5')
    joins.save(WORK / 'repeat-joins.jpg',quality=94)
    print(WORK / 'review.html')


if __name__ == '__main__':
    main()
