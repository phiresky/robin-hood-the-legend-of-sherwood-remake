"""Freeze review-only fill inputs; never create approval or generation metadata."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

import numpy as np
from PIL import Image


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(worker, expected_model, output):
    model = worker / 'model.blend'
    if sha(model) != expected_model:
        raise ValueError('Private geometry revision changed')
    packet = worker / 'modified'
    frames = json.loads((packet / 'views.json').read_text())
    if [v['index'] for v in frames['views']] != list(range(8)):
        raise ValueError('Expected eight ordered fixed cameras')
    width, height = frames['tile_size']
    canvas = width * 4, height * 2
    if (any(v % 16 for v in canvas) or max(canvas) > 3840
            or max(canvas) / min(canvas) > 3 or not 655360 <= canvas[0] * canvas[1] <= 8294400):
        raise ValueError('Source dimensions need a separately reviewed render; never resize')
    evidence = {str(model): expected_model}
    for path, digest in frames['source_mask_evidence'].items():
        if sha(Path(path)) != digest:
            raise ValueError('Source ownership evidence changed: ' + path)
        evidence[path] = digest
    prepared = []
    inputs = Image.new('RGBA', canvas)
    solids = Image.new('RGBA', canvas)
    masks = Image.new('RGBA', canvas, 'white')
    for view in frames['views']:
        index = view['index']
        paths = {kind: packet / 'views' / f'view-{index}-{kind}.png'
                 for kind in ('known', 'solid', 'textured')}
        if sha(paths['known']) != view['ownership_sha256']:
            raise ValueError('Ownership buffer changed')
        images = {kind: Image.open(path).convert('RGBA') for kind, path in paths.items()}
        if any(image.size != (width, height) for image in images.values()):
            raise ValueError('Fixed camera tile dimensions changed')
        unknown = ((np.asarray(images['solid'])[:, :, 3] > 0)
                   & ~(np.asarray(images['known'])[:, :, 0] > 127))
        pixels = np.full((height, width, 4), 255, dtype=np.uint8)
        pixels[unknown, 3] = 0
        mask = Image.fromarray(pixels)
        position = index % 4 * width, index // 4 * height
        inputs.paste(images['textured'], position)
        solids.paste(images['solid'], position)
        masks.paste(mask, position)
        prepared.append((index, paths, mask, int(unknown.sum())))
        evidence.update({str(path): sha(path) for path in paths.values()})
    for image, filename in [(inputs, 'textured.png'), (solids, 'solid.png')]:
        if not np.array_equal(np.asarray(image), np.asarray(Image.open(packet / filename).convert('RGBA'))):
            raise ValueError('Assembled input differs from frozen sheet')
        evidence[str(packet / filename)] = sha(packet / filename)
    output.mkdir(parents=True, exist_ok=False)
    (output / 'views').mkdir()
    for index, paths, mask, _ in prepared:
        for kind, path in paths.items():
            shutil.copyfile(path, output / 'views' / path.name)
        mask.save(output / 'views' / f'view-{index}-mask.png')
    inputs.save(output / 'input.png')
    solids.save(output / 'solid.png')
    masks.save(output / 'mask.png')
    shutil.copyfile(packet / 'views.json', output / 'reviewed-cameras.json')
    evidence[str(packet / 'views.json')] = sha(packet / 'views.json')
    if sha(model) != expected_model:
        raise ValueError('Geometry changed during preparation')
    report = dict(status='private inputs only; user geometry approval pending',
                  asset_id=frames['asset_id'], worker=str(worker), model_sha256=expected_model,
                  size=list(canvas), editable_pixels=sum(row[3] for row in prepared),
                  source_evidence=evidence, geometry_modified=False,
                  approval_created=False, generation_authorized=False,
                  files={str(path.relative_to(output)): sha(path)
                         for path in output.rglob('*') if path.is_file()})
    (output / 'private-inputs.json').write_text(json.dumps(report, indent=2) + '\n')
    print(output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    parser.add_argument('expected_model')
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    prepare(args.worker.resolve(), args.expected_model, args.output.resolve())
