"""Freeze a tree40 donor from its individual crown rather than its shared mask."""
import json
from pathlib import Path
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from catalog import OUT, tree_workspace
from evidence_io import sha, write_json


def main():
    worker = tree_workspace(40)
    output = OUT / 'texture-fill-round-1/croisement02-tree-40/native-synthesis-scoped-v2'
    validate_only = '--validate' in sys.argv
    if output.exists() and not validate_only:
        raise FileExistsError(output)
    report = json.loads((worker / 'inspection/refinement.json').read_text())
    partition = Path(report['source_packet'])
    packet = json.loads(partition.read_text())
    rgba_path = Path(packet['lobes'][0]['image'])
    rgba = np.asarray(Image.open(rgba_path).convert('RGBA'))
    x0, y0, width, height = packet['native_bbox']
    source_path = worker / 'reference/source.png'
    source = np.asarray(Image.open(source_path).convert('RGB'))
    if rgba.shape[:2] != (height, width):
        raise ValueError('Individual partition dimensions changed')
    if not np.array_equal(rgba[:, :, :3], source[y0:y0+height, x0:x0+width]):
        raise ValueError('Individual crown RGB is not exact map source')
    owned = rgba[:, :, 3] > 127
    ys, xs = np.where(owned)
    left, top, right, bottom = xs.min(), ys.min(), xs.max()+1, ys.max()+1
    crop = [int(x0+left), int(y0+top), int(x0+right), int(y0+bottom)]
    if validate_only:
        donor = np.asarray(Image.open(output / 'donor.png').convert('RGB'))
        mask = np.asarray(Image.open(output / 'donor-mask.png').convert('L')) > 0
        if not np.array_equal(donor, source[crop[1]:crop[3],crop[0]:crop[2]]) or not np.array_equal(mask, owned[top:bottom,left:right]):
            raise ValueError('Donor differs from exact individual source partition')
        allowed = set(map(tuple, donor[mask].tolist()))
        tile = np.asarray(Image.open(output / 'tile.png').convert('RGB'))
        if any(tuple(pixel) not in allowed for pixel in tile.reshape(-1,3).tolist()):
            raise ValueError('Synthesis used pixels outside the individual crown')
        conditioned = output / 'conditioned'
        canvas = np.asarray(Image.open(conditioned / 'native-canvas.png').convert('RGB'))
        keep = np.asarray(Image.open(conditioned / 'known-mask.png').convert('L')) > 0
        continuation = np.asarray(Image.open(conditioned / 'continuation.png').convert('RGB'))
        if not np.array_equal(canvas[keep], continuation[keep]):
            raise ValueError('Continuation changed protected native pixels')
        if any(tuple(pixel) not in allowed for pixel in continuation.reshape(-1,3).tolist()):
            raise ValueError('Continuation used foreign pixels')
        write_json(output / 'donor-validation.json', dict(status='PASS', source_sha256=sha(source_path),
            mask_source=str(rgba_path), mask_source_sha256=sha(rgba_path), source_crop=crop,
            all_donor_pixels_exact_source=True, source_owned_samples=int(mask.sum()),
            individual_partition=str(partition), individual_partition_sha256=sha(partition),
            foreign_donor_used=False, bark_donor_used=False,
            files={name:sha(output/name) for name in ['donor.png','donor-mask.png','tile.png','donor-provenance.json']}))
        print('Exact individual donor and conditioned continuation PASS')
        return
    output.mkdir()
    Image.fromarray(source[crop[1]:crop[3], crop[0]:crop[2]]).save(output / 'donor.png')
    Image.fromarray(owned[top:bottom, left:right].astype('uint8')*255).save(output / 'donor-mask.png')
    write_json(output / 'donor-provenance.json', dict(role='foliage', source=str(source_path),
        source_sha256=sha(source_path), crop=crop, native_domain_pixels=int(owned.sum()),
        individual_partition=str(partition), individual_partition_sha256=sha(partition),
        partition_image=str(rgba_path), partition_image_sha256=sha(rgba_path),
        approved_model_sha256=sha(worker / 'model.blend'),
        scope='Exact individual tree40 crown alpha. Shared native mask132 alone is insufficient.',
        supersedes='native-synthesis-v1 donor had6061 shared-mask samples, only687 inside this individual crown.'))
    # Keep the already observed boundary and synthesize only its continuation.
    origin = [1664, 256]
    canvas = np.zeros((384, 384, 3), dtype=np.uint8)
    keep = np.zeros((384, 384), dtype=bool)
    for yy, xx in zip(ys, xs):
        dx, dy = int(x0+xx-origin[0]), int(y0+yy-origin[1])
        if not (0 <= dx < 384 and 0 <= dy < 384):
            raise ValueError('Crown does not fit continuation canvas')
        canvas[dy, dx] = rgba[yy, xx, :3]
        keep[dy, dx] = True
    conditioned = output / 'conditioned'
    conditioned.mkdir()
    Image.fromarray(canvas).save(conditioned / 'native-canvas.png')
    Image.fromarray(keep.astype('uint8')*255).save(conditioned / 'known-mask.png')
    write_json(conditioned / 'source-provenance.json', dict(source=str(source_path),
        source_sha256=sha(source_path), native_mask=str(rgba_path), native_mask_sha256=sha(rgba_path),
        individual_partition=str(partition), individual_partition_sha256=sha(partition),
        origin=origin, dimensions=[384,384], accepted=int(keep.sum()),
        own_partition_bbox=packet['bbox'], canvas_sha256=sha(conditioned/'native-canvas.png'),
        mask_sha256=sha(conditioned/'known-mask.png')))
    print(output)


if __name__ == '__main__':
    main()
