"""Patch only a saved flight's cubic translation values and tangents along source rays."""
import argparse
import hashlib
import json
import math
import os
import struct
from pathlib import Path

os.environ['OPENBLAS_NUM_THREADS'] = '2'
os.environ['OMP_NUM_THREADS'] = '2'

import numpy as np
from scipy.interpolate import CubicSpline


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--old-inputs', type=Path, required=True)
    parser.add_argument('--new-inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assert not args.output.exists()
    original = args.model.read_bytes()
    size = struct.unpack_from('<I', original, 12)[0]
    doc = json.loads(original[20:20 + size])
    binary = bytearray(original[28 + size:])

    def accessor(index):
        a = doc['accessors'][index]
        view = doc['bufferViews'][a['bufferView']]
        assert a['componentType'] == 5126 and 'sparse' not in a
        width = {'SCALAR': 1, 'VEC3': 3}[a['type']]
        assert view.get('byteStride', 4 * width) == 4 * width
        return np.ndarray((a['count'], width), '<f4', binary,
                          view.get('byteOffset', 0) + a.get('byteOffset', 0))

    inputs = [json.loads(p.read_text()) for p in (args.old_inputs, args.new_inputs)]
    curves = [d['continuous_height_curve'] for d in inputs]
    assert curves[0]['times'] == curves[1]['times']
    splines = [CubicSpline(c['times'], c['heights'], bc_type='periodic') for c in curves]
    channels = [(a, c) for a in doc['animations'] for c in a['channels']
                if c['target']['path'] == 'translation']
    assert len(channels) == 1
    animation, channel = channels[0]
    assert channel['target']['node'] == 0
    sampler = animation['samplers'][channel['sampler']]
    assert sampler['interpolation'] == 'CUBICSPLINE'
    seconds = accessor(sampler['input']).ravel().astype(float)
    phases = np.round(seconds * 100) / 8
    assert np.max(np.abs(seconds - phases * .08)) < 1e-6
    assert np.array_equal(phases, np.asarray(curves[0]['times']))
    values = accessor(sampler['output']).reshape(-1, 3, 3)
    delta = splines[1](phases) - splines[0](phases)
    velocity = (splines[1](phases, 1) - splines[0](phases, 1)) / .08
    axis = np.array([0., 1., 1 / math.tan(math.radians(35))])
    values[:, 1] += delta[:, None] * axis
    values[:, 0] += velocity[:, None] * axis
    values[:, 2] += velocity[:, None] * axis
    assert abs(delta[0]) < 1e-10 and abs(delta[-1]) < 1e-10
    output_accessor = doc['accessors'][sampler['output']]
    output_accessor['min'] = values.reshape(-1, 3).min(0).astype(float).tolist()
    output_accessor['max'] = values.reshape(-1, 3).max(0).astype(float).tolist()
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    payload = struct.pack('<4sII', b'glTF', 2, 28 + len(encoded) + len(binary))
    payload += struct.pack('<I4s', len(encoded), b'JSON') + encoded
    payload += struct.pack('<I4s', len(binary), b'BIN\0') + binary
    args.output.mkdir()
    model = args.output / args.model.name
    model.write_bytes(payload)
    changed = np.flatnonzero(np.frombuffer(original[28 + size:], np.uint8)
                             != np.frombuffer(binary, np.uint8))
    view = doc['bufferViews'][output_accessor['bufferView']]
    assert all(view['byteOffset'] <= i < view['byteOffset'] + view['byteLength'] for i in changed)
    report = dict(status='PRIVATE_TRANSLATION_ONLY_PATCH_PENDING_READBACK',
                  original_model_sha256=sha(args.model), model_sha256=sha(model),
                  old_inputs_sha256=sha(args.old_inputs), new_inputs_sha256=sha(args.new_inputs),
                  translation_keys=len(phases), changed_binary_bytes=len(changed),
                  retained='All images, materials, geometry, hierarchy, rotations and timing bytes exact; only translation output and its bounds updated.',
                  maximum_knot_height_delta=float(max(abs(delta))))
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    main()
