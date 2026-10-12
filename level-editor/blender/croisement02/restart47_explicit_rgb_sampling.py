"""Declare discrete projected RGB sampling without changing approved binary payloads."""
import copy
import hashlib
import json
import pathlib
import struct

ROOT = pathlib.Path(__file__).resolve().parents[2]
BASE = ROOT / 'work/active/croisement02'
OUT = BASE / 'restart47-explicit-rgb-sampling-v1'


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def pin(path):
    return {'path': str(path), 'sha256': sha(path)}


def save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    assert not path.exists(), path
    path.write_text(json.dumps(data, indent=2) + '\n')


replacements = {}
models = []


def register(old, new):
    replacements[str(old)] = str(new)
    replacements[sha(old)] = sha(new)


def rewrite(value):
    if isinstance(value, str):
        return replacements.get(value, value)
    if isinstance(value, list):
        return [rewrite(v) for v in value]
    if isinstance(value, dict):
        return {k: rewrite(v) for k, v in value.items()}
    return value


def model(old, new, holes):
    raw = old.read_bytes()
    assert raw[:4] == b'glTF' and struct.unpack_from('<I', raw, 8)[0] == len(raw)
    size, kind = struct.unpack_from('<II', raw, 12)
    assert kind == 0x4e4f534a
    original = json.loads(raw[20:20 + size])
    doc = copy.deepcopy(original)
    selected = []
    for i, material in enumerate(doc['materials']):
        extra = material.get('extras', {})
        if extra.get('export_complementary_binary_ownership') and (holes or extra['export_composite_role'] == 'native'):
            assert 'emissiveTexture' in material and 'export_rgb_sampling' not in extra
            extra['export_rgb_sampling'] = 'nearest-required'
            selected.append(i)
    assert len(selected) == (2 if holes else 67)
    undo = copy.deepcopy(doc)
    for i in selected:
        del undo['materials'][i]['extras']['export_rgb_sampling']
    assert undo == original
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    suffix = raw[20 + size:]
    result = struct.pack('<4sII', b'glTF', 2, 20 + len(encoded) + len(suffix)) + struct.pack('<II', len(encoded), kind) + encoded + suffix
    new.parent.mkdir(parents=True, exist_ok=True)
    assert not new.exists()
    new.write_bytes(result)
    assert new.read_bytes()[20 + len(encoded):] == suffix
    register(old, new)
    models.append({'source': pin(old), 'model': pin(new), 'flagged_materials': selected, 'all_binary_chunks_unchanged': True, 'json_except_explicit_sampling_flag_unchanged': True})


def config(old, new):
    data = json.loads(old.read_text())
    for binding in data['bindings']:
        prior = pathlib.Path(binding['contract']['path'])
        target = new.parent / prior.name
        save(target, rewrite(json.loads(prior.read_text())))
        register(prior, target)
    updated = rewrite(data)
    updated['asset_handle']['root_directory'] = str(new.parent)
    updated['sampling_revision'] = pin(pathlib.Path(__file__))
    save(new, updated)
    register(old, new)


assert not OUT.exists(), OUT
for phase in ('initial', 'applied'):
    model(BASE / 'restart38-approved-hole-export-v1' / phase / 'model.glb', OUT / 'holes' / phase / 'model.glb', True)
for i in range(20):
    site = f'site-{i:02d}'
    model(BASE / 'restart40-approved-mound-export-v2' / site / 'model.glb', OUT / 'mounds' / site / 'model.glb', False)
config(BASE / 'restart38-approved-hole-export-v1/reader-config.json', OUT / 'holes/reader-config.json')
config(BASE / 'restart40-approved-mound-export-v2/reader-config.json', OUT / 'mounds/reader-config.json')
integration = BASE / 'restart39-current-hole-apertures-v1/integration-focused-v2.json'
save(OUT / 'holes/integration-focused.json', rewrite(json.loads(integration.read_text())))
save(OUT / 'manifest.json', {'status': 'PRIVATE_EXPLICIT_RGB_SAMPLING_BROWSER_PENDING', 'models': models, 'hole_config': pin(OUT / 'holes/reader-config.json'), 'hole_integration': pin(OUT / 'holes/integration-focused.json'), 'mound_config': pin(OUT / 'mounds/reader-config.json'), 'scope': 'Both complementary hole branches require nearest RGB after browser-proven padding bleed. Mound native projections retain authored closest sampling; inferred RGB remains smooth.', 'source_approval_unchanged': True, 'canonical_writes': False, 'recipe': pin(pathlib.Path(__file__))})
print(json.dumps(pin(OUT / 'manifest.json')))
