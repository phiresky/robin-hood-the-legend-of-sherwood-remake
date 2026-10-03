"""Build riverbank art from Day/sherwood.map.png using Pillow and texture-synthesis.

Run from any directory: python3 synthesize-riverbanks.py /path/to/Data/Levels/Day
"""
import argparse
import base64
import json
import math
from pathlib import Path
import random
import shutil
import subprocess
import zlib
from PIL import Image, ImageDraw, ImageFilter, ImageEnhance

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('maps', type=Path)
parser.add_argument('--synthesizer', default=shutil.which('texture-synthesis'))
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
work = root / 'work/riverbanks'
work.mkdir(parents=True, exist_ok=True)
out = root / 'app/src/terrain-textures'
source = Image.open(args.maps / 'sherwood.map.png').convert('RGB')
# Bounds in the full-resolution day map; no mission markers or interface pixels.
donors = {'soil': (1590, 303, 1660, 357), 'plant': (1470, 513, 1540, 572)}
for i, (name, box) in enumerate(donors.items()):
    donor = source.crop(box)
    donor = donor.resize((donor.width, round(donor.height / math.sin(math.radians(35)))))
    donor.save(work / f'{name}-donor.png')
    subprocess.run([args.synthesizer, '--no-progress', '--tiling', '--threads', '1',
                    '--seed', str(281 + i), '--out-size', '128x512', '--out', str(work / f'{name}.png'),
                    'generate', str(work / f'{name}-donor.png')], check=True)
# Preserve a painted boulder's silhouette and lighting rather than drawing synthetic stones.
rock = source.crop((1330, 195, 1410, 254)).convert('RGBA')
mask = Image.new('L', rock.size)
ImageDraw.Draw(mask).polygon([(5, 47), (10, 28), (27, 9), (45, 2), (65, 10), (76, 27),
                             (78, 43), (55, 54), (24, 58)], fill=255)
rock.putalpha(mask.filter(ImageFilter.GaussianBlur(0.7)))
rock = rock.resize((80, 94), Image.Resampling.LANCZOS)
soil = ImageEnhance.Brightness(ImageEnhance.Color(Image.open(work / 'soil.png').convert('RGB')).enhance(0.55)).enhance(0.65).convert('RGBA')
plant = Image.open(work / 'plant.png').convert('RGBA')
packed = {}
contact = Image.new('RGB', (6 * 128, 512))
for column, kind in enumerate(['plain', 'small_stones', 'big_stones', 'mixed_stones', 'stones_plants', 'vegetation']):
    tile = soil.copy()
    rng = random.Random(395 + column)
    if kind in ['stones_plants', 'vegetation']:
        mask = Image.new('L', tile.size)
        draw = ImageDraw.Draw(mask)
        for _ in range(80 if kind == 'vegetation' else 32):
            x, y = rng.randrange(128), rng.randrange(512)
            rx, ry = rng.randrange(9, 26), rng.randrange(12, 36)
            for dy in [-512, 0, 512]:
                draw.ellipse((x-rx, y-ry+dy, x+rx, y+ry+dy), fill=255)
        tile = Image.composite(plant, tile, mask.filter(ImageFilter.GaussianBlur(3)))
    if kind not in ['plain', 'vegetation']:
        count = 115 if kind in ['small_stones', 'stones_plants'] else 35 if kind == 'big_stones' else 70
        for _ in range(count):
            size = rng.randint(7, 17) if kind in ['small_stones', 'stones_plants'] else rng.randint(25, 47) if kind == 'big_stones' else rng.randint(8, 43)
            stamp = rock.resize((size, round(size * 1.17)), Image.Resampling.LANCZOS)
            # Small rotation variations retain the painted lighting direction.
            stamp = stamp.rotate(rng.uniform(-18, 18), expand=True, resample=Image.Resampling.BICUBIC)
            x, y = rng.randrange(128), rng.randrange(512)
            for dx in [-128, 0, 128]:
                for dy in [-512, 0, 512]:
                    tile.alpha_composite(stamp, (x + dx - stamp.width // 2, y + dy - stamp.height // 2))
    tile = tile.convert('RGB').quantize(colors=256)
    contact.paste(tile.convert('RGB'), (column * 128, 0))
    packed[kind] = {'width':128, 'height':512, 'palette':base64.b64encode(bytes(tile.getpalette())).decode(),
                    'pixelsZlib':base64.b64encode(zlib.compress(tile.tobytes(), 9)).decode()}
(out / 'riverbanks.json').write_text(json.dumps(packed, separators=(',', ':')) + '\n')
contact.save(out / 'riverbanks.png')
print('Wrote riverbank textures and review swatch', flush=True)
