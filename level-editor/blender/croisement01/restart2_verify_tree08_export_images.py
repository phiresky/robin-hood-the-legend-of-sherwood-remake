"""Compare embedded GLB RGBA images against the exact approved saved worker."""
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
import bpy
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R = ROOT/'level-editor/work/active/croisement01/restart2'
model = R/'approved-tree08-bark-fill-v3/croisement01-tree-08/baked-v8-connected-gaps/worker.blend'
out = R/'tree08-approved-wood-export-v2'
glb = out/'assets/croisement01-tree-08/model.glb'
assert sha(model)=='01dedc44df9d06962a76b6bbf6d4fc96715f219ee0af7eb782ba22677892767a'
assert sha(glb)=='02802ad1b5a91fadbd2e69222a64c314d89a015529a1230caaa00f8dbec5cbda'
def pixels(data):
    image = Image.open(io.BytesIO(data)).convert('RGBA')
    return dict(size=list(image.size),rgba_sha256=hashlib.sha256(image.tobytes()).hexdigest())
acquire(slots=2)
try:
    bpy.ops.wm.open_mainfile(filepath=str(model))
    expected = {im.name:pixels(im.packed_file.data) for im in bpy.data.images if im.packed_file}
    raw = glb.read_bytes()
    n = struct.unpack_from('<I',raw,12)[0]
    document = json.loads(raw[20:20+n])
    binary = raw[28+n:]
    rows=[]
    for material in document['materials']:
        binding = material['pbrMetallicRoughness'].get('baseColorTexture',material.get('emissiveTexture'))
        assert binding is not None, material['name']
        texture = binding['index']
        image = document['images'][document['textures'][texture]['source']]
        view = document['bufferViews'][image['bufferView']]
        start=view.get('byteOffset',0)
        actual=pixels(binary[start:start+view['byteLength']])
        matches=[name for name,value in expected.items() if value==actual]
        assert matches, material['name']
        rows.append(dict(material=material['name'],approved_image_matches=matches,**actual))
    assert len(rows)==3
    (out/'embedded-image-proof.json').write_text(json.dumps(dict(status='ALL_THREE_EMBEDDED_RGBA_IMAGES_EXACT',model_sha256=sha(model),glb_sha256=sha(glb),images=rows),indent=2)+'\n')
    print('All three embedded images match exact approved RGBA',flush=True)
finally:
    release()
