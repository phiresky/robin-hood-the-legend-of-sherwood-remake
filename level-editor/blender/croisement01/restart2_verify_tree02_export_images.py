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
model = R/'ready-tree02-texture-v3/model.blend'
out = R/'tree02-approved-integration-v1'
glb = out/'assets/croisement01-tree-02/model.glb'
assert sha(model)=='0436e5e2ad8ca3109fc2aeeaafa223f2d0c90cb5eea694bec0dcd2409ea1408f'
assert sha(glb)=='48676ff322a34c44c6c258402a9b44587b1d7a29bbd1570ead4adeff9e3f7751'
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
        texture = material['pbrMetallicRoughness']['baseColorTexture']['index']
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
