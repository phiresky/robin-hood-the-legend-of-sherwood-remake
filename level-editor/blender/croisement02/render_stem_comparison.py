"""Compare old and new stem geometry using identical cameras and studio light."""
import argparse
import json
from pathlib import Path
import sys

import bpy
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from evidence_io import sha, write_json
from render_multiview_asset import render
from render_slots import acquire, release


def main(worker):
    output = worker / 'inspection/baseline-comparison'
    assert not output.exists(), 'Preserve previous comparison evidence'
    manifest = worker / 'inspection/actual-camera-manifest.json'
    frames = json.loads(manifest.read_text())
    width, height = frames['tile_size']
    before = {name: sha(worker / name) for name in ('model.blend', 'baseline.blend')}
    acquire()
    try:
        for label, file in [('previous', 'baseline.blend'), ('candidate', 'model.blend')]:
            bpy.ops.wm.open_mainfile(filepath=str(worker / file))
            scene = bpy.data.scenes[frames['scene_name']]
            shading = scene.display.shading
            shading.light = 'STUDIO'
            shading.studio_light = 'paint.sl'
            shading.color_type = 'SINGLE'
            shading.single_color = (.6, .6, .6)
            shading.show_cavity = False
            shading.show_shadows = True
            shading.show_specular_highlight = False
            shading.show_object_outline = False
            render(manifest, output / label, modes=('solid',), width=width)
        for start in (0, 4):
            sheet = Image.new('RGB', (width * 4, (height + 24) * 2), '#151515')
            draw = ImageDraw.Draw(sheet)
            for j in range(4):
                for row, label in enumerate(('previous', 'candidate')):
                    x, y = j * width, row * (height + 24)
                    draw.text((x + 4, y + 4), f'{start + j}: ' + ('previous approved geometry' if row == 0 else 'NEW joined candidate'), fill='white')
                    sheet.paste(Image.open(output / label / f'view-{start + j}-solid.png').convert('RGB'), (x, y + 24))
            sheet.save(output / f'comparison-{start}-{start + 3}.png')
        assert before == {name: sha(worker / name) for name in before}
        write_json(output / 'evidence.json', dict(model_sha256=before['model.blend'], baseline_sha256=before['baseline.blend'],
            camera_manifest_sha256=sha(manifest), layout='Previous above/new below; identical cameras and neutral studio shading, cavity emphasis disabled.',
            sheets={p.name: sha(p) for p in output.glob('comparison-*.png')}))
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    main(parser.parse_args(sys.argv[sys.argv.index('--') + 1:]).worker.resolve())
