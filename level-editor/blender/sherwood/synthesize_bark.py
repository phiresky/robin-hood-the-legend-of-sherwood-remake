"""Build the seamless bark donor with the editor's texture-synthesis CLI."""
import os
import subprocess
from pathlib import Path
from PIL import Image
from paths import DATA, OUT

OUT.mkdir(parents=True, exist_ok=True)
donor = OUT/'oak-bark-broad-donor.png'
with Image.open(DATA/'Levels/Day/sherwood.map.png') as source:
    source.crop((931, 499, 1006, 585)).convert('RGB').save(donor)
subprocess.run([
    os.environ.get('TEXTURE_SYNTHESIS', str(Path.home()/'.cargo/bin/texture-synthesis')),
    '--tiling', '--threads', '8', '--seed', '58', '--out-size', '128x512',
    '--out', str(OUT/'oak-bark-seamless.png'), 'generate', str(donor),
], check=True)
