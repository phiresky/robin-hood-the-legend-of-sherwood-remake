"""Pin approved endpoint cameras and corrected gatehouse context for one browser owner."""
from pathlib import Path
import hashlib
import json
import struct

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/york-refinement/restart2'
OUT = BASE / 'restart44-approved-portcullis-export-v1'


def pin(path):
    return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


def views(path):
    rows = json.loads(path.read_text())['renders']
    assert len(rows) == 8
    return [{'index': i, 'camera_matrix_world': row['camera_matrix'],
             'ortho_scale': row['ortho_scale'], 'resolution': row['resolution'],
             'reference': pin(Path(row['file']))} for i, row in enumerate(rows)]


def main():
    handoff = OUT / 'handoff.json'
    assert pin(handoff)['sha256'] == '214ab7e82ec3a494928db8f259a64d226a102cb994d90cb0efd62cd112987d74'
    gatehouse = BASE / 'restart43-approved-gatehouse-export-v1'
    context_request = json.loads((gatehouse / 'browser-request.json').read_text())
    authority = json.loads((BASE / 'approved-gate-staging-plan-v1/plan.json').read_text())
    for state, context_state in [('covered', 'initial'), ('raised', 'applied')]:
        folder = OUT / state
        source = BASE / 'gate-textures-v1' / state
        report = json.loads((folder / 'export-report.json').read_text())
        member = next(x for x in authority['approved_members'] if x['asset_id'] == 'york-castle-portcullis--' + state)
        assert report['source_model_sha256'] == member['model_sha256'] == pin(source / 'model.blend')['sha256']
        model = folder / '3d-assets/york-castle-portcullis/model.glb'
        assert pin(model)['sha256'] == report['model_sha256']
        raw = model.read_bytes()
        doc = json.loads(raw[20:20 + struct.unpack_from('<I', raw, 12)[0]])
        cameras = source / 'actual/renders.json'
        context = BASE / 'restart42-channel-assembly-v1/complete-review-v1' / context_state
        request = {
            'status': 'READY_FOR_SINGLE_BROWSER_OWNER', 'kind': 'PRIVATE_APPROVED_YORK_STATIC_ASSET',
            'asset_id': 'york-castle-portcullis', 'state': state,
            'integration_handoff': pin(handoff), 'model': pin(model),
            'descriptor': pin(model.with_name('asset.json')),
            'source_surface_guard': pin(folder / 'source-surface-guard.json'),
            'scene_names': [s.get('name') for s in doc['scenes']],
            'mesh_names': [n['name'] for n in doc['nodes'] if 'mesh' in n],
            'part_names': [n['name'] for n in doc['nodes'] if 'editor_part_node' in n.get('extras', {})],
            'placement_origin_scene': report['source_origin_scene'],
            'camera_space': 'Saved cameras are world scene-space Z-up. Exported vertices subtract the common pivot; GLB basis wrapper converts Z-up to Y-up. Apply production basis once.',
            'raised_geometry_already_contains_lift': state == 'raised',
            'placement_rule': 'Both endpoints use the identical origin. The raised asset already contains its 69.584151559403 world-Z lift; never translate it by that lift again.',
            'saved_camera_manifest': pin(cameras), 'reference_sheet': pin(source / 'actual/textured-native-labeled.png'),
            'native_comparison': pin(source / 'contact-review/native-comparison.png'),
            'views': views(cameras),
            'context': {'model': context_request['model'], 'descriptor': context_request['descriptor'],
                        'placement_origin_scene': context_request['placement_origin_scene'],
                        'saved_camera_manifest': pin(context / 'renders.json'),
                        'reference_sheet': pin(context / 'textured-eight.png'),
                        'views': views(context / 'renders.json'),
                        'scope': 'Corrected static gatehouse plus this endpoint. Gatehouse context remains at its own pivot; no moving winch included.'},
            'checks': ['Load exact pinned GLB through production loader/display.',
                       'Inspect eight saved approved endpoint views; original source camera first/top-left.',
                       'Inspect combined corrected gatehouse and endpoint context.',
                       'Preserve opaque ownership alpha semantics, unlit atlas and culling.',
                       'Verify endpoint positions coincide with their saved source cameras; do not double-lift raised endpoint.'],
            'limitations': ['Isolated endpoint/context verification does not establish production moving-pose integration, collision, audio, full editor save/reload or publication.'],
            'canonical_writes': False,
        }
        output = folder / 'browser-request.json'
        assert not output.exists()
        output.write_text(json.dumps(request, indent=2) + '\n')
        print(state, pin(output))


if __name__ == '__main__':
    main()
