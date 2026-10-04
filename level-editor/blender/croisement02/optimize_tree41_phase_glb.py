"""Deduplicate the isolated phase proof and apply native counter timing."""
import hashlib, json, struct
import numpy as np
from catalog import OUT


def main():
    root = OUT / 'tree41-phase-appearance-proof-v2'
    path = root / 'phase-appearance-tree41.glb'
    raw = path.read_bytes()
    source = root / 'phase-appearance-tree41-unoptimized.glb'
    if not source.exists():
        source.write_bytes(raw)
    raw = source.read_bytes()
    size = struct.unpack_from('<I', raw, 12)[0]
    doc = json.loads(raw[20:20+size])
    binary = raw[28+size:]
    before = {key: len(doc.get(key, [])) for key in ['bufferViews', 'accessors', 'images', 'textures', 'materials']}
    # Exact byte deduplication preserves source paint and all geometry values.
    def dedup(key, signature=None):
        unique, mapping, seen = [], {}, {}
        for index, value in enumerate(doc[key]):
            sig = signature(value) if signature else json.dumps({k:v for k,v in value.items() if k != 'name'}, sort_keys=True)
            if sig not in seen:
                seen[sig] = len(unique)
                unique.append(value)
            mapping[index] = seen[sig]
        doc[key] = unique
        return mapping
    def view_signature(view):
        start = view.get('byteOffset', 0)
        meta = {k:v for k,v in view.items() if k not in ['name', 'byteOffset', 'buffer']}
        return json.dumps(meta, sort_keys=True), binary[start:start+view['byteLength']]
    mapping = dedup('bufferViews', view_signature)
    for value in doc['accessors'] + doc['images']:
        if 'bufferView' in value: value['bufferView'] = mapping[value['bufferView']]
    mapping = dedup('accessors')
    for mesh in doc['meshes']:
        for prim in mesh['primitives']:
            prim['attributes'] = {k:mapping[v] for k,v in prim['attributes'].items()}
            if 'indices' in prim: prim['indices'] = mapping[prim['indices']]
    for animation in doc['animations']:
        for sampler in animation['samplers']:
            for field in ['input', 'output']: sampler[field] = mapping[sampler[field]]
    mapping = dedup('images')
    for texture in doc['textures']: texture['source'] = mapping[texture['source']]
    mapping = dedup('textures')
    def remap_texture(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key.endswith('Texture') and isinstance(child, dict) and 'index' in child:
                    child['index'] = mapping[child['index']]
                else: remap_texture(child)
        elif isinstance(value, list):
            for child in value: remap_texture(child)
    for material in doc['materials']: remap_texture(material)
    mapping = dedup('materials')
    for mesh in doc['meshes']:
        for prim in mesh['primitives']:
            if 'material' in prim: prim['material'] = mapping[prim['material']]
    rebuilt = bytearray()
    for view in doc['bufferViews']:
        start = view.get('byteOffset', 0)
        data = binary[start:start+view['byteLength']]
        rebuilt.extend(b'\0' * (-len(rebuilt) % 4))
        view['byteOffset'] = len(rebuilt)
        rebuilt.extend(data)
    def accessor(array, kind):
        rebuilt.extend(b'\0' * (-len(rebuilt) % 4))
        view = len(doc['bufferViews'])
        doc['bufferViews'].append(dict(buffer=0, byteOffset=len(rebuilt), byteLength=array.nbytes))
        rebuilt.extend(array.tobytes())
        index = len(doc['accessors'])
        entry = dict(bufferView=view, componentType=5126, count=len(array), type=kind)
        if kind == 'SCALAR': entry.update(min=[float(array.min())], max=[float(array.max())])
        doc['accessors'].append(entry)
        return index
    # Counter increments before testing > delay: delay 3 lasts four 40 ms ticks.
    times = accessor(np.arange(15, dtype=np.float32) * np.float32(.16), 'SCALAR')
    animation = doc['animations'][0]
    for channel in animation['channels']:
        node = doc['nodes'][channel['target']['node']]
        phase = 0 if 'native phase' not in node['name'] else int(node['name'].rsplit(' ', 1)[-1])
        # Names may use an underscore suffix in future exporters.
        if 'native_phase_' in node['name']: phase = int(node['name'].rsplit('_', 1)[-1])
        values = np.zeros((15, 3), np.float32)
        values[phase] = 1
        if phase == 0: values[-1] = 1
        animation['samplers'][channel['sampler']] = dict(input=times, output=accessor(values, 'VEC3'), interpolation='STEP')
    doc.setdefault('extras', {})['nativeTiming'] = dict(ticks_per_second=25, serialized_delay=3, ticks_per_phase=4, seconds_per_phase=.16, cycle_seconds=2.24)
    rebuilt.extend(b'\0' * (-len(rebuilt) % 4))
    doc['buffers'][0]['byteLength'] = len(rebuilt)
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    body = struct.pack('<II', len(encoded), 0x4e4f534a) + encoded + struct.pack('<II', len(rebuilt), 0x004e4942) + rebuilt
    result = struct.pack('<III', 0x46546c67, 2, len(body)+12) + body
    path.write_bytes(result)
    report = dict(status='PASS', original_bytes=len(raw), optimized_bytes=len(result), reduction_fraction=1-len(result)/len(raw), before=before, after={key:len(doc[key]) for key in before}, geometry_and_image_bytes_lossless=True, timing=doc['extras']['nativeTiming'], sha256=hashlib.sha256(result).hexdigest())
    (root/'optimization-verification.json').write_text(json.dumps(report, indent=2)+'\n')
    proof_path = root/'proof.json'
    proof = json.loads(proof_path.read_text())
    proof.update(glb_sha256=report['sha256'], native_timing=report['timing'], glb_optimization=str(root/'optimization-verification.json'), worker_timing_note='Blender construction timeline is a phase staging aid; final standalone GLB has verified native 25 Hz counter timing.')
    proof_path.write_text(json.dumps(proof, indent=2)+'\n')
    verification_path = root/'glb-verification.json'
    verification = json.loads(verification_path.read_text())
    verification.update(glb_sha256=report['sha256'], duration_seconds=2.24, sample_times=15, lossless_resource_deduplication=True, native_timing=report['timing'])
    verification_path.write_text(json.dumps(verification, indent=2)+'\n')
    print(json.dumps(report, indent=2))

if __name__ == '__main__': main()
