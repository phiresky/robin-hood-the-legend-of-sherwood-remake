"""Audit completed private texture bakes without claiming visual approval."""
import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
from PIL import Image


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(path.read_text())


def verify_protected_rgba(source, result, mask):
    if mask.mode == 'RGBA':
        editable = np.asarray(mask)[:, :, 3] == 0
    elif mask.mode == 'L':
        editable = np.asarray(mask) > 127
    else:
        raise ValueError('Unsupported authoritative edit-mask mode: ' + mask.mode)
    if source.shape != result.shape or source.shape[:2] != editable.shape:
        raise ValueError('Protected RGBA dimensions differ')
    if not np.any(editable) or not np.any(~editable):
        raise ValueError('Expected both editable and protected pixels')
    if np.any(source[~editable] != result[~editable]):
        raise ValueError('Protected RGBA bytes differ')
    return int(np.count_nonzero(editable))


def audit(experiment):
    experiment = experiment.resolve()
    bake = experiment / 'baked-preserved-v1'
    validation = read(bake / 'validation.json')
    if validation.get('geometry_verified') is not True:
        raise ValueError('Saved bake did not verify approved geometry')
    review = read(experiment / 'prebake-review.json')
    if review['status'] != 'suitable-for-guarded-bake':
        raise ValueError('Generation is held')
    generation = experiment / review['generation']
    preserved = generation / 'generated-preserved.png'
    if sha(preserved) != review['preserved_sha256']:
        raise ValueError('Generation changed after prebake inspection')
    if read(generation / 'generation.json')['changedProtected'] != 0:
        raise ValueError('Generation changed protected source pixels')
    source = np.array(Image.open(experiment / 'input.png').convert('RGBA'))
    result = np.array(Image.open(preserved).convert('RGBA'))
    verify_protected_rgba(source, result, Image.open(experiment / 'mask.png'))
    launch = read(experiment / 'bake-launch.json')
    if sha(launch['recipe']) != launch['recipe_sha256']:
        raise ValueError('Archived launch recipe changed')
    manifest = read(experiment / 'views.json')
    first = manifest['views'][0]
    direction = np.array(first['camera_matrix_world'])[:3, 2]
    expected = np.array([0, -math.cos(math.radians(35)), math.sin(math.radians(35))])
    if first['index'] != 0 or not np.allclose(direction, expected, atol=1e-5):
        raise ValueError('Top-left camera is not the native artwork direction')
    receipt = {
        'asset_id': manifest['asset_id'],
        'status': 'Mechanical checks complete; actual eight-view self-review and independent review pending',
        'texture_approved': False,
        'baked_model': str(bake / 'worker.blend'),
        'baked_model_sha256': sha(bake / 'worker.blend'),
        'actual_sheet': str(bake / 'actual/textured.png'),
        'actual_sheet_sha256': sha(bake / 'actual/textured.png'),
        'validation_sha256': sha(bake / 'validation.json'),
        'protected_rgba_changes': 0,
        'native_camera_tile': 0,
        'executed_recipe': launch,
        'completion_recipe_receipt_sha256': sha(bake / 'inspection-recipe.json'),
        'recipe_provenance_note': 'Use the archived launch source above as execution evidence. The completion receipt hashes the file present at completion, which may include a later queue guard.',
    }
    destination = bake / 'mechanical-receipt.json'
    if destination.exists() and read(destination) != receipt:
        raise ValueError('Existing mechanical receipt differs; preserve it and investigate')
    destination.write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiment', type=Path)
    print(json.dumps(audit(parser.parse_args().experiment), indent=2))
