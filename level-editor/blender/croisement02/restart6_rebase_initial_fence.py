"""Prepare a private intact-fence endpoint without changing any mesh or image payload."""
from pathlib import Path
import hashlib
import json
import math
import struct


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


base = Path('level-editor/work/croisement02-refinement')
stage = base / 'restart2-textures/batch10-private-level3d-exact-v2'
pins_path = stage / 'placement-evidence/static-placement-pins.json'
pins = json.loads(pins_path.read_text())
assert sha(pins_path) == '555ef8c32146799c059a72e77b7f7586b16fd5a41d9624381259736cfa6ee266'
rows = pins['fence']
assert {row['object_id'] for row in rows} == {'building-019', 'building-020'}
transform = rows[0]['group_transform']
for row in rows:
    assert row['transform'] == {'dx': 0, 'dy': 0, 'dz': 0, 'rot_deg': 0}
    assert row['group_transform'] == transform
assert transform['rot_deg'] == 0
asset = next(row for row in pins['asset_sources']
             if row['id'] == 'croisement02-south-field-wattle-fence')
source = stage / 'map-assets' / asset['model']
assert sha(source) == asset['model_sha256'] and not asset['resources']
anchor_file = base / 'restart2-state/remaining-local-origins-v1/manifest.json'
anchor = json.loads(anchor_file.read_text())['anchors']['south-field-fence']
# The placed Z-up translation becomes [x, z, -y] in the glTF frame.
angle = math.radians(35)
placed = [transform['dx'], transform['dz'] / math.cos(angle),
          transform['dy'] / math.sin(angle)]
translation = [placed[i] - anchor[i] for i in range(3)]
raw = source.read_bytes()
magic, version, total = struct.unpack_from('<III', raw)
assert magic == 0x46546c67 and version == 2 and total == len(raw)
chunks = []
offset = 12
while offset < len(raw):
    length, kind = struct.unpack_from('<II', raw, offset)
    chunks.append((kind, raw[offset+8:offset+8+length]))
    offset += 8 + length
assert offset == len(raw) and chunks[0][0] == 0x4e4f534a
model = json.loads(chunks[0][1])
original_nodes = json.loads(json.dumps(model['nodes']))
scene_index = next(i for i, scene in enumerate(model['scenes'])
                   if scene.get('name') == asset['model_scene'])
old_roots = model['scenes'][scene_index]['nodes']
model['nodes'].append({'name': 'Common fence endpoint origin',
                       'translation': translation, 'children': old_roots})
model['scenes'][scene_index]['nodes'] = [len(model['nodes']) - 1]
model['scene'] = scene_index
assert model['nodes'][:-1] == original_nodes
encoded = json.dumps(model, separators=(',', ':')).encode()
encoded += b' ' * ((-len(encoded)) % 4)
new_chunks = [(chunks[0][0], encoded), *chunks[1:]]
body = b''.join(struct.pack('<II', len(data), kind) + data for kind, data in new_chunks)
out = base / 'restart2-state/initial-fence-family-origin-v1'
out.mkdir(exist_ok=True)
destination = out / 'initial-fence.glb'
result = struct.pack('<III', magic, version, 12 + len(body)) + body
if destination.exists():
    assert destination.read_bytes() == result
else:
    destination.write_bytes(result)
reopened = destination.read_bytes()
json_length = struct.unpack_from('<I', reopened, 12)[0]
assert reopened[20+json_length:] == raw[20+len(chunks[0][1]):]
error = max(abs(anchor[i] + translation[i] - placed[i]) for i in range(3))
assert error < 1e-12
receipt = {'status': 'PRIVATE_EQUIVALENT_REBASE', 'model': str(destination),
           'model_sha256': sha(destination), 'source_model': str(source),
           'source_model_sha256': sha(source), 'placement_pins_sha256': sha(pins_path),
           'position': anchor, 'wrapper_translation': translation,
           'placed_y_up_translation': placed, 'restored_translation_error': error,
           'original_nodes_preserved': True, 'all_binary_chunks_byte_exact': True,
           'static_replacements': rows,
           'scope': 'Exact private export at common intact/cleared family origin. '
                    'No geometry/material change, static publication or gameplay claim.'}
(out / 'manifest.json').write_text(json.dumps(receipt, indent=2) + '\n')
print(json.dumps(receipt))
