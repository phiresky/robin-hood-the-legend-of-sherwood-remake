"""Reconcile historical floor obligations against exact installed atlas bytes."""
import hashlib
import io
import json
import struct
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement02-refinement'
OUT = BASE / 'static-terrain-closure-oct11-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def atlas(path):
    raw = path.read_bytes()
    length = struct.unpack_from('<I', raw, 12)[0]
    doc = json.loads(raw[20:20 + length])
    binary = raw[28 + length:]
    view = doc['bufferViews'][doc['images'][0]['bufferView']]
    start = view.get('byteOffset', 0)
    png = binary[start:start + view['byteLength']]
    return doc, binary, png, np.array(Image.open(io.BytesIO(png)).convert('RGBA'))


def main():
    OUT.mkdir(exist_ok=True)
    floor = BASE / 'restart4-final-floor-bake-v1'
    first = BASE / 'restart2-textures/approved206-ground-export-v1'
    second = BASE / 'restart2-textures/approved14-ground-export-v1'
    last = BASE / 'restart25-approved-appearance-delta-v1/terrain-fringe49-v1'
    installed = ROOT / 'level-editor/library/3d-assets/croisement02/croisement02-terrain/model.glb'
    paths = [first / 'model.glb', second / 'model.glb', last / 'model.glb', installed]
    images = [atlas(p)[3] for p in paths]
    validations = [json.loads((p / 'validation.json').read_text()) for p in (first, second)]
    final = json.loads((last / 'verification.json').read_text())
    assert sha(paths[0]) == validations[0]['output_sha256'] == validations[1]['source_sha256']
    assert sha(paths[1]) == validations[1]['output_sha256'] == final['source_glb_sha256']
    assert sha(paths[2]) == sha(installed) == final['candidate_glb_sha256']
    assert np.array_equal(images[0], np.array(Image.open(floor / 'composite.png').convert('RGBA')))
    assert hashlib.sha256(atlas(installed)[2]).hexdigest() == final['approved_composite_sha256']
    receipts = [floor / 'user-appearance-approval.json', BASE / 'restart4-fence14-ground-candidate-v1/user-appearance-approval.json', BASE / 'restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json']
    assert [sha(p) for p in receipts] == [validations[0]['approval_sha256'], validations[1]['approval_sha256'], final['approval_sha256']]
    for receipt in receipts[:2]:
        data = json.loads(receipt.read_text())
        receipt_path = Path(data.get('receipt', data.get('batch_receipt')))
        if not receipt_path.is_absolute():
            receipt_path = ROOT / receipt_path
        assert sha(receipt_path) == data.get('receipt_sha256', data.get('batch_receipt_sha256'))
    changes = [np.any(a != b, axis=2) for a, b in zip(images, images[1:])]
    assert [int(m.sum()) for m in changes] == [14, 49, 0]
    assert all(np.array_equal(images[0][:, :, 3], im[:, :, 3]) for im in images[1:])
    residual_path = BASE / 'restart4-ground-closure-inventory-v1/remaining5930.png'
    residual = np.array(Image.open(residual_path).convert('L')) > 0
    domain_path = floor / 'combined-domain.png'
    domain = np.array(Image.open(domain_path).convert('L')) > 0
    assert int(residual.sum()) == 5930 and int(domain.sum()) == 575810
    assert np.all(domain[residual])
    pins = paths + receipts + [floor / 'composite.png', domain_path, residual_path, first / 'validation.json', second / 'validation.json', last / 'verification.json']
    report = {
        'status': 'PASS_SCOPED_INSTALLED_FLOOR_APPEARANCE_CHAIN',
        'scope': 'Historical 5930-pixel floor backlog and 569880-pixel underlying bank domain, plus subsequent approved 14 and 49 pixel corrections. Not foreground anatomy, elevated bank appearance, state behavior or whole-map completion.',
        'installed_sha256': sha(installed),
        'approved_floor_domain_pixels': int(domain.sum()),
        'historical_residual_pixels': int(residual.sum()),
        'installed_matches_approved_floor_on_residual': int(np.all(images[0] == images[-1], axis=2)[residual].sum()),
        'later_approved_changed_residual_pixels': int(np.any(images[0] != images[-1], axis=2)[residual].sum()),
        'later_approved_pixel_changes': [int(m.sum()) for m in changes],
        'alpha_exact': True,
        'model_writes': 0,
        'pins': {str(p.relative_to(ROOT)): sha(p) for p in pins},
        'remaining': ['Independent current foreground/source-ownership and contact closure; old neutral-floor ledger is not a live missing-fill queue.'],
    }
    (OUT / 'approval-chain-audit.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'pins'}, indent=2))


if __name__ == '__main__':
    main()
