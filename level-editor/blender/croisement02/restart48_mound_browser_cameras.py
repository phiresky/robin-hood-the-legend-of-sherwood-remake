"""Prepare isolated mound camera frames from approved directions and exact exported positions."""
import copy
import hashlib
import json
from pathlib import Path
import struct
import numpy as np

BASE = Path(__file__).resolve().parents[2] / 'work/active/croisement02'
OUT = BASE / 'restart48-mound-browser-cameras-v1'
MODELS = BASE / 'restart47-explicit-rgb-sampling-v1/mounds'
TEMPLATE = BASE / 'restart25-approved-state-materialization-v1/mound-filled-all-sites-v5/isolated-review-v1/views.json'


def pin(path):
    return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


def save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    assert not path.exists(), path
    path.write_text(json.dumps(data, indent=2) + '\n')


def positions(path):
    raw = path.read_bytes()
    length = struct.unpack_from('<I', raw, 12)[0]
    doc = json.loads(raw[20:20 + length])
    buffer = raw[28 + length:]
    points = []
    for node in doc['nodes']:
        if 'mesh' not in node:
            continue
        assert not any(k in node for k in ['matrix', 'translation', 'rotation', 'scale'])
        for primitive in doc['meshes'][node['mesh']]['primitives']:
            accessor = doc['accessors'][primitive['attributes']['POSITION']]
            assert accessor['componentType'] == 5126 and accessor['type'] == 'VEC3'
            view = doc['bufferViews'][accessor['bufferView']]
            assert view.get('byteStride', 12) == 12 and view.get('buffer', 0) == 0
            offset = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
            xyz = np.frombuffer(buffer, '<f4', count=accessor['count'] * 3, offset=offset).reshape(-1, 3).astype(float)
            points.append(np.column_stack([xyz[:, 0], -xyz[:, 2], xyz[:, 1]]))
    result = np.concatenate(points)
    assert np.isfinite(result).all()
    return result


assert not OUT.exists()
template = json.loads(TEMPLATE.read_text())
assert len(template['views']) == 8 and template['views'][0]['index'] == 0
reference = positions(MODELS / 'site-00/model.glb')
reference_center = (reference.min(axis=0) + reference.max(axis=0)) / 2
cards = []
for i in range(20):
    site = f'site-{i:02d}'
    model = MODELS / site / 'model.glb'
    xyz = positions(model)
    lower, upper = xyz.min(axis=0), xyz.max(axis=0)
    center = (lower + upper) / 2
    delta = center - reference_center
    views = []
    for source in template['views']:
        matrix = np.array(source['camera_matrix_world'])
        matrix[:3, 3] += delta
        local = (xyz - matrix[:3, 3]) @ matrix[:3, :3]
        required = 2 * float(np.abs(local[:, :2]).max()) * 1.05
        scale = max(float(source['ortho_scale']), required)
        # Translation and optional framing enlargement cannot alter the approved direction.
        assert np.array_equal(matrix[:3, :3], np.array(source['camera_matrix_world'])[:3, :3])
        assert float(np.abs(local[:, :2]).max()) < scale / 2
        views.append({'index': source['index'], 'azimuth_degrees': source['azimuth_degrees'], 'camera_matrix_world': matrix.tolist(), 'camera_location': matrix[:3, 3].tolist(), 'camera_rotation_euler': source['camera_rotation_euler'], 'ortho_scale': scale, 'source_ortho_scale': source['ortho_scale'], 'framing_enlarged': scale > float(source['ortho_scale']) + 1e-8, 'maximum_screen_fraction': float(np.abs(local[:, :2]).max()) / (scale / 2), 'label': 'Original art direction' if source['index'] == 0 else f"Approved direction {source['index']}"})
    manifest = {'version': 1, 'asset_id': 'croisement02-hiding-mound-' + site, 'scene_name': site + '-initial', 'tile_size': template['tile_size'], 'layout': template['layout'], 'views': views, 'camera_coordinate_frame': 'Blender world Z-up; convert cameras to GLTF Y-up or rotate scene +90 degrees X exactly once.', 'derivation': 'Approved site00 camera rotations and scales translated by exact exported AABB center difference. Scale enlarged only if required to keep all vertices inside frame with 5 percent padding.', 'template': pin(TEMPLATE), 'model': pin(model), 'position_evidence': {'reference_model': pin(MODELS / 'site-00/model.glb'), 'reference_center_zup': reference_center.tolist(), 'aabb_min_zup': lower.tolist(), 'aabb_max_zup': upper.tolist(), 'center_zup': center.tolist(), 'camera_translation_from_template_zup': delta.tolist()}, 'source_direction_top_left': True, 'model_pivot_absolute_yup': [0, 0, 0]}
    target = OUT / site / 'views.json'
    save(target, manifest)
    cards.append({'site': site, 'model': pin(model), 'scene': site + '-initial', 'pivot_absolute_yup': [0, 0, 0], 'camera_manifest': pin(target), 'all_eight_frames_contain_every_vertex': True})
request = {'status': 'CPU_READY_ALL20_ISOLATED_MOUND_VIEWS_BROWSER_PENDING', 'cards': cards, 'reader_config': pin(MODELS / 'reader-config.json'), 'verification': pin(MODELS / 'verification.json'), 'sampling_manifest': pin(BASE / 'restart47-explicit-rgb-sampling-v1/manifest.json'), 'template': pin(TEMPLATE), 'approved_appearance': pin(BASE / 'restart25-approved-state-materialization-v1/mound-texture-ready-v1/handoff.json'), 'runtime_commit': '477b2dade', 'scope': '20 isolated initial mound shapes,160 views;32 control lifecycle and ground context remain additional checks.', 'canonical_writes': False, 'recipe': pin(Path(__file__))}
save(OUT / 'browser-request.json', request)
print(json.dumps(pin(OUT / 'browser-request.json')))
