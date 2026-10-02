"""Extract representative existing texture payloads without re-encoding them."""
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
map_name = sys.argv[1] if len(sys.argv) > 1 else None
OUT = ROOT / 'work/bc7f-benchmark' / ('inputs-' + map_name if map_name else 'inputs')
OUT.mkdir(parents=True, exist_ok=True)
models = [
    'york/york-bridge-square-central-timber-house',
    'york/york-terrain',
    'leicester/leicester-watermill',
    'leicester/leicester-southeast-cottage-tree',
    'sherwood/sherwood-spreading-oak',
]
if map_name:
    manifest = json.loads((ROOT / 'library/scenes' / (map_name + '.rhlos-map.json')).read_text())
    models = list(dict.fromkeys(str(Path(ref['model']).relative_to('3d-assets').parent)
                               for ref in manifest['sceneAssets'] + manifest['assetSources']))
rows = []
for model in models:
    path = ROOT / 'library/3d-assets' / model / 'lossy.glb'
    if not path.exists():
        path = path.with_name("model.glb")
    data = path.read_bytes()
    n = struct.unpack_from('<I', data, 12)[0]
    doc = json.loads(data[20:20+n])
    for index, image in enumerate(doc.get('images', [])):
        view = doc['bufferViews'][image['bufferView']]
        offset = 28 + n + view.get('byteOffset', 0)
        payload = data[offset:offset+view['byteLength']]
        decoded = Image.open(io.BytesIO(payload))
        name = model.split('/')[-1] + f'-{index}'
        (OUT / name).write_bytes(payload)
        rows.append(dict(name=name, width=decoded.width, height=decoded.height,
                         mime=image['mimeType'], bytes=len(payload),
                         sha256=hashlib.sha256(payload).hexdigest()))
(OUT / 'manifest.json').write_text(json.dumps(rows, indent=2) + '\n')
print(f'Extracted {len(rows)} textures; {sum(r["width"]*r["height"] for r in rows):,} pixels; {sum(r["bytes"] for r in rows):,} encoded bytes')
