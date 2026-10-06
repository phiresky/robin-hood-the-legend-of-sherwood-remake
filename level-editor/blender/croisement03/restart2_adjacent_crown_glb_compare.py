"""Bind standalone pre-fill crown conversion to the actual filled derivative."""
import hashlib
import json
import struct
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
B = ROOT / 'level-editor/work/croisement03-refinement/restart2'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    data = path.read_bytes()
    length = struct.unpack_from('<I', data, 12)[0]
    return json.loads(data[20:20 + length]), data[28 + length:]


def accessor(doc, binary, index):
    a = doc['accessors'][index]
    v = doc['bufferViews'][a['bufferView']]
    dtype = {5121: '<u1', 5123: '<u2', 5125: '<u4', 5126: '<f4'}[a['componentType']]
    width = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
    offset = v.get('byteOffset', 0) + a.get('byteOffset', 0)
    item = np.dtype(dtype).itemsize
    return np.ndarray((a['count'], width), dtype=dtype, buffer=binary, offset=offset,
                      strides=(v.get('byteStride', item * width), item)).copy()


def material(doc, binary, index):
    m = json.loads(json.dumps(doc['materials'][index]))
    m.pop('name', None)

    def replace(value):
        if not isinstance(value, dict):
            return
        for key, child in value.items():
            if key.endswith('Texture') and isinstance(child, dict):
                texture = doc['textures'][child.pop('index')]
                image = doc['images'][texture['source']]
                view = doc['bufferViews'][image['bufferView']]
                start = view.get('byteOffset', 0)
                child['image_sha256'] = sha(binary[start:start + view['byteLength']])
                child['sampler'] = doc['samplers'][texture['sampler']]
            else:
                replace(child)
    replace(m)
    return m


def main():
    out = B / 'adjacent-crown-renderer-audit-v1'
    rows = []
    for tree in (12, 14):
        before = out / f'export-identity-v2/tree{tree}-before.glb'
        after = B / f'tree{tree}-exact-export-v2/model.glb'
        a, ab = read(before)
        b, bb = read(after)
        am = a['meshes'][0]
        bm = next(m for m in b['meshes'] if m['name'] == am['name'])
        assert len(am['primitives']) == len(bm['primitives']) == 1
        ap, bp = am['primitives'][0], bm['primitives'][0]
        assert ap['attributes'].keys() == bp['attributes'].keys()
        errors = {}
        for key in ap['attributes']:
            av, bv = accessor(a, ab, ap['attributes'][key]), accessor(b, bb, bp['attributes'][key])
            assert av.shape == bv.shape
            if key == 'POSITION':
                delta = bv.astype(float) - av.astype(float)
                error = float(np.max(np.abs(delta - delta[0])))
                assert error < .0002, error
                errors[key] = dict(max_reframing_residual=error, constant_translation=delta[0].tolist())
            elif key == 'NORMAL':
                error = float(np.max(np.abs(av.astype(float) - bv.astype(float))))
                assert error <= .000101, error
                errors[key] = dict(max_component_error=error, bound=.000101,
                                   note='Small measured export difference; physical foliage is unlit.')
            else:
                assert np.array_equal(av, bv), key
                errors[key] = dict(exact=True)
        assert np.array_equal(accessor(a, ab, ap['indices']), accessor(b, bb, bp['indices']))
        ma, mb = material(a, ab, ap['material']), material(b, bb, bp['material'])
        assert ma == mb, (ma, mb)
        rows.append(dict(tree=tree, baseline_sha256=sha(before.read_bytes()), actual_derivative_sha256=sha(after.read_bytes()),
                         attributes=errors, indices_exact=True, material_sampler_rgba_alpha_exact=True,
                         physical_material=ma))
    target = out / 'actual-derivative-crown-proof.json'
    assert not target.exists()
    target.write_text(json.dumps(dict(status='PASS actual derivative uses unchanged pre-fill physical crown', trees=rows,
        limitation='The tiny position residual is float32 origin reframing. The browser/Blender appearance difference remains visible and disclosed; this proves no changed crown shader/alpha/UV/texture from bark filling.'), indent=2) + '\n')


if __name__ == '__main__':
    main()
