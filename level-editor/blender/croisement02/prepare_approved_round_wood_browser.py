"""Pin private tree deliveries and the frozen approval cameras for loader review."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement02-refinement'
OUT = BASE / 'approved-round-wood-integration-oct11-v1/browser-request-v1'


def pin(path):
    path = path.resolve()
    return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    cards = []
    for n in (32, 38):
        integration = OUT.parent / f'tree-{n}-v1'
        delivery = integration / 'delivery-v1'
        export = json.loads((integration / 'export.json').read_text())
        x, y, z = export['pivot']
        context = BASE / f'tree{n}-root-research/approved-wood-texture-context-oct11-v1'
        experiment = BASE / f'approved-round-wood-textures-oct11-v1/tree{n}/experiment'
        cameras = json.loads((context / 'views.json').read_text())
        assert len(cameras['views']) == 8
        assert cameras['views'][0]['azimuth_degrees'] == 0
        card = {
            'site': f'tree-{n}', 'asset_id': f'croisement02-tree-{n}',
            'model': pin(delivery / 'model.glb'), 'scene': 'default',
            'lossy': pin(delivery / 'lossy.glb'),
            'preview': pin(delivery / 'preview.glb'),
            'descriptor': pin(delivery / 'asset.json'),
            'lossy_receipt': pin(delivery / 'lossy.glb.receipt.json'),
            'preview_receipt': pin(delivery / 'preview.glb.receipt.json'),
            'pivot_absolute_yup': [x, z, -y],
            'pivot_absolute_blender_zup': [x, y, z],
            'camera_manifest': pin(context / 'views.json'),
            'wood_close_camera_manifest': pin(experiment / 'views.json'),
            'reference_context_sheet': pin(context / 'textured.png'),
            'reference_wood_sheet': pin(experiment / 'original-atlas-bake-v2/actual/textured.png'),
            'reference_context_evidence': pin(context / 'evidence.json'),
            'source_surface_guard': pin(integration / 'source-surface-guard.json'),
            'merge_proof': pin(integration / 'merge-proof.json'),
            'delivery_proof': pin(integration / 'delivery-proof.json'),
            'framing_scope': 'Frozen approved wood/context cameras. Full crown fit is not asserted. Ground in reference sheet is diagnostic context, absent from isolated GLB.',
            'approval_scope': 'Wood appearance and geometry only; installed crown retained exactly.'
        }
        cards.append(card)
    request = {
        'status': 'PRIVATE_DELIVERIES_CPU_VERIFIED_BROWSER_PENDING',
        'cards': cards,
        'camera_convention': 'Frozen matrices are absolute Blender Z-up, row-major. Convert camera world basis and location by (x,y,z)->(x,z,-y). Either place GLB root at pivot_absolute_yup and retain absolute camera, or subtract that pivot from converted camera position with model at origin. Do not rotate GLB again: its map wrapper already converts Z-up to Y-up.',
        'camera_order': 'Eight frozen views, original art camera first/top-left.',
        'approval': pin(BASE / 'restart3-review-batches/six-scoped-reviews-oct11-v1/user-approval.json'),
        'resource_delta': pin(OUT.parent / 'private-resource-delta.json'),
        'external_resources': [pin(p) for p in sorted((OUT.parent / 'blobs').glob('*')) if p.is_file()],
        'scope': 'Actual application loader review of exact private model, lossy derivative and preview. Compare wood close views and crown context, check texture availability and root placement. No canonical publication.',
        'canonical_writes': False,
        'recipe': pin(Path(__file__)),
    }
    (OUT / 'browser-request.json').write_text(json.dumps(request, indent=2) + '\n')
    print(json.dumps(pin(OUT / 'browser-request.json')))


if __name__ == '__main__':
    main()
