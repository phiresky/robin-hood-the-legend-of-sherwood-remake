"""Assemble current evidence for manual inspection, without granting readiness."""
import argparse
import json
import sys
from pathlib import Path
from PIL import Image, ImageDraw
from catalog import OUT, tree_workspace

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'refinement/blender'))
from evidence_io import sha, write_json


def assemble(mask):
    w = tree_workspace(mask)
    model_hash = sha(w / 'model.blend')
    for path in ['inspection/refinement.json', 'inspection/actual-materials/evidence.json',
                 'inspection/source-coverage/report.json', 'inspection/saved-model-audit.json']:
        if json.loads((w / path).read_text())['model_sha256'] != model_hash:
            raise ValueError('Evidence is stale: ' + str(w / path))
    destination = OUT / 'self-review-round-2' / f'tree-{mask:02}'
    destination.mkdir(parents=True, exist_ok=True)
    files = {}
    # Keep native eight-view sheet resolution: image scaling is only used to
    # enlarge the small source comparison, never to hide a model defect.
    actual = Image.open(w / 'inspection/actual-materials/sheet.png').convert('RGB')
    source_paths = ['source.png', 'render.png', 'difference.png']
    row_height = max(Image.open(w / 'inspection/source-coverage' / f).height for f in source_paths)
    width = max(actual.width, sum(Image.open(w / 'inspection/source-coverage' / f).width for f in source_paths))
    board = Image.new('RGB', (width, actual.height+row_height+64), '#454545')
    draw = ImageDraw.Draw(board)
    draw.text((8, 4), f'Tree {mask:02}: saved materials, all eight views', fill='white')
    board.paste(actual, (0, 24))
    x = 0
    for name in source_paths:
        path = w / 'inspection/source-coverage' / name
        image = Image.open(path).convert('RGBA')
        draw.text((x+4, actual.height+28), name, fill='white')
        board.paste(image, (x, actual.height+48), image)
        x += image.width
        files[str(path)] = sha(path)
    board.save(destination / 'actual-and-source.png')
    solid, textured = [Image.open(w / 'modified' / f).convert('RGB') for f in ('solid.png', 'textured.png')]
    board = Image.new('RGB', (solid.width, solid.height+textured.height+48), '#454545')
    draw = ImageDraw.Draw(board)
    draw.text((8, 4), f'Tree {mask:02}: solid, all eight views', fill='white')
    board.paste(solid, (0,24))
    draw.text((8, solid.height+28), 'Source-only ownership, all eight views', fill='white')
    board.paste(textured, (0, solid.height+48))
    board.save(destination / 'solid-and-source-only.png')
    for path in ['modified/solid.png', 'modified/textured.png', 'inspection/actual-materials/sheet.png']:
        files[str(w / path)] = sha(w / path)
    if sha(w / 'model.blend') != model_hash:
        raise ValueError('Model changed while assembling evidence')
    write_json(destination / 'packet.json', dict(model_sha256=model_hash, files=files,
               sheets={name: sha(destination/name) for name in ['actual-and-source.png', 'solid-and-source-only.png']}))
    print(destination)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('masks', nargs='+', type=int)
    for mask in parser.parse_args().masks:
        assemble(mask)
