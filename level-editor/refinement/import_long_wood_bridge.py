"""Import the CC BY 4.0 Long Wood Bridge download into the local editor catalog.

Source: https://sketchfab.com/3d-models/long-wood-bridge-e2b094603d0a44c8bf5a94a899e5c01c
Credit: Horus Chen (embedded author), currently listed as Kogeniku on Sketchfab.
Accepts either supplied texture resolution; preserves image payloads and geometry detail.
After importing, generate browser derivatives from the repository root with:
    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/refinement/blender/lossy_assets.py -- library \
      --root level-editor/library/3d-assets --assets sketchfab-long-wood-bridge \
      --run level-editor/work/bridge-lossy-import --apply
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from asset_index import write_asset_index

ASSET_ID = 'sketchfab-long-wood-bridge'
SOURCE = 'https://sketchfab.com/3d-models/long-wood-bridge-e2b094603d0a44c8bf5a94a899e5c01c'
SOURCES = {
    '689a54dd6c619ee3a9b836aaa10ea6e2539c263697636766ea83f7c5b16bc3b6': 2048,
    'ea2a7c808deec7cb6f46448ef3d0e8066dfdbbbd13ddfadd1373410228e26d26': 1024,
}


def import_bridge(source, library, scale=10.0):
    data = source.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if digest not in SOURCES:
        raise ValueError('Unrecognized bridge download; inspect its geometry before importing')
    size = struct.unpack_from('<I', data, 12)[0]
    model = json.loads(data[20:20 + size])
    binary = bytearray(data[28 + size:])
    # The verified download has one mesh and inverse parent rotations that cancel.
    # Bake its Y-up geometry into the editor part's Z-up frame, keeping the group identity.
    primitive = model['meshes'][0]['primitives'][0]
    floor = model['accessors'][primitive['attributes']['POSITION']]['min'][1]
    for semantic in ('POSITION', 'NORMAL', 'TANGENT'):
        if semantic not in primitive['attributes']:
            continue
        accessor = model['accessors'][primitive['attributes'][semantic]]
        dimensions = 4 if semantic == 'TANGENT' else 3
        if accessor['componentType'] != 5126:
            raise ValueError('Expected floating-point bridge geometry')
        view = model['bufferViews'][accessor['bufferView']]
        start = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
        stride = view.get('byteStride', dimensions * 4)
        values = []
        for i in range(accessor['count']):
            offset = start + i * stride
            vector = struct.unpack_from('<' + 'f' * dimensions, binary, offset)
            x, y, z = vector[:3]
            transformed = (x * scale, -z * scale, (y - floor) * scale) if semantic == 'POSITION' else (x, -z, y)
            struct.pack_into('<' + 'f' * dimensions, binary, offset, *transformed, *vector[3:])
            values.append(struct.unpack_from('<fff', binary, offset))
        if semantic == 'POSITION':
            accessor['min'] = [min(p[i] for p in values) for i in range(3)]
            accessor['max'] = [max(p[i] for p in values) for i in range(3)]
    model['nodes'] = [
        {'name': 'map', 'rotation': [-2 ** -0.5, 0, 0, 2 ** -0.5], 'children': [1]},
        {'name': 'Long Wood Bridge', 'extras': {'asset_group': ASSET_ID}, 'children': [2]},
        {'name': 'scenery-long-wood-bridge', 'mesh': 0, 'extras': {'scenery': True, 'part_name': 'Long Wood Bridge'}},
    ]
    model['scenes'] = [{'name': 'default', 'nodes': [0]}]
    model['scene'] = 0
    encoded = json.dumps(model, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    binary += b'\0' * (-len(binary) % 4)
    glb = struct.pack('<4sII', b'glTF', 2, 28 + len(encoded) + len(binary))
    glb += struct.pack('<I4s', len(encoded), b'JSON') + encoded
    glb += struct.pack('<I4s', len(binary), b'BIN\0') + binary
    positions = model['accessors'][primitive['attributes']['POSITION']]
    descriptor = {
        'version': 1, 'kind': 'projection-mapped-asset', 'id': ASSET_ID,
        'name': 'Long Wood Bridge', 'source_map': 'Sketchfab', 'asset_type': 'Bridge',
        'tags': ['bridge', 'wood', 'timber', 'long', 'low-poly', 'cc-by-4.0', 'kogeniku', 'horus-chen'],
        'model': 'model.glb', 'model_scene': 'default', 'resources': [],
        'parts': [{'node': 'scenery-long-wood-bridge', 'name': 'Long Wood Bridge', 'scenery': True}],
        'bounds_local_scene': {'min': positions['min'], 'max': positions['max']},
        'provenance': {
            'source': SOURCE, 'source_sha256': digest, 'texture_resolution': SOURCES[digest],
            'author': model['asset']['extras']['author'], 'current_author': 'Kogeniku (https://sketchfab.com/kogeniku)',
            'license': 'CC-BY-4.0', 'license_url': 'https://creativecommons.org/licenses/by/4.0/',
            'changes': 'Converted Y-up to Z-up, scaled by %g, grounded and wrapped for the editor; geometry detail and textures retained.' % scale,
            'scene_units_per_source_unit': scale,
        },
    }
    output = library / 'sketchfab' / ASSET_ID
    output.mkdir(parents=True, exist_ok=False)
    (output / 'model.glb').write_bytes(glb)
    (output / 'asset.json').write_text(json.dumps(descriptor, indent=2) + '\n')
    (output / 'ATTRIBUTION.txt').write_text(
        'Long Wood Bridge\nBy Horus Chen (embedded credit); currently Kogeniku on Sketchfab.\n'
        + SOURCE + '\nLicensed under CC BY 4.0: https://creativecommons.org/licenses/by/4.0/\n'
        + descriptor['provenance']['changes'] + '\n')
    write_asset_index(library)
    print(json.dumps({'asset': str(output), 'bytes': len(glb), 'bounds': descriptor['bounds_local_scene']}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('--library', type=Path, default=Path(__file__).resolve().parents[1] / 'library/3d-assets')
    args = parser.parse_args()
    import_bridge(args.source, args.library)
