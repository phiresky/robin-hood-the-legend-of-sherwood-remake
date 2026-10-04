"""Compare original approved materials with a fill under identical bake cameras."""
import argparse
import json
from pathlib import Path
import sys
import bpy
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from bake_texture_candidate import acquire, release, preflight, snapshot, require, sha
from render_multiview_asset import render


def main(experiment, bake, output):
    require(not output.exists(), 'Comparison output exists')
    acquire()
    try:
        manifest, scene, names, before, _ = preflight(experiment)
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.transparent_max_bounces = 64
        render(experiment / 'views.json', output, width=manifest['tile_size'][0])
        require(snapshot(scene, names) == before, 'Baseline rendering changed approved scene')
        width, height = manifest['tile_size']
        original = Image.new('RGBA', (width * 4, height * 2))
        for i in range(8):
            original.paste(Image.open(output / f'view-{i}-textured.png').convert('RGBA'), ((i % 4) * width, (i // 4) * height))
        original.save(output / 'original-textured.png')
        for start in (0, 4):
            sheet = Image.new('RGB', (width * 4, (height + 24) * 2), '#141414')
            draw = ImageDraw.Draw(sheet)
            for j in range(4):
                i = start + j
                for row, path in [(0, output / f'view-{i}-textured.png'), (1, bake / 'actual' / f'view-{i}-textured.png')]:
                    x, y = j * width, row * (height + 24)
                    draw.text((x + 5, y + 5), f'{i}: ' + ('original approved materials' if row == 0 else 'generated fill'), fill='white')
                    sheet.paste(Image.open(path).convert('RGB'), (x, y + 24))
            sheet.save(output / f'comparison-{start}-{start+3}.png')
        report = dict(asset_id=manifest['asset_id'], approved_model_sha256=sha(experiment / 'approved-model.blend'),
            baked_model_sha256=sha(bake / 'worker.blend'), camera_manifest_sha256=sha(experiment / 'views.json'),
            rendering='Same approved camera transforms/scales, Cycles 8 samples, transparent_max_bounces64 and original scene lighting for baseline and bake.',
            artifacts={str(p.relative_to(output)): sha(p) for p in sorted(output.rglob('*')) if p.is_file()})
        (output / 'comparison.json').write_text(json.dumps(report, indent=2) + '\n')
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiment', type=Path)
    parser.add_argument('bake', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.experiment.resolve(), args.bake.resolve(), args.output.resolve())
