"""Private source-donor fill of unknown physical foliage; never touches bark."""
import argparse
from array import array
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from bake_texture_candidate import preflight, snapshot, require, sha, acquire, release
from fill_physical_foliage import fill
from refinement_review import _tile
from render_multiview_asset import render


def main(experiment, donor_dir, output):
    require(not output.exists(), 'Use a fresh candidate directory')
    evidence = {str(p): sha(p) for p in [donor_dir / 'donor.png', donor_dir / 'donor-mask.png',
                donor_dir / 'tile.png', donor_dir / 'donor-provenance.json', donor_dir / 'donor-validation.json']}
    from PIL import Image
    provenance = json.loads((donor_dir / 'donor-provenance.json').read_text())
    validation = json.loads((donor_dir / 'donor-validation.json').read_text())
    require(validation['status'] == 'PASS', 'Native donor ownership has not passed validation')
    require(sha(Path(provenance['source'])) == provenance['source_sha256'], 'Native source changed')
    require(sha(Path(validation['mask_source'])) == validation['mask_source_sha256'], 'Native mask changed')
    require(all(sha(donor_dir / name) == expected for name, expected in validation['files'].items()),
            'Native donor validation became stale')
    source = np.array(Image.open(donor_dir / 'donor.png').convert('RGB'))
    require(np.array_equal(source, np.array(Image.open(provenance['source']).convert('RGB').crop(provenance['crop']))),
            'Donor is not an exact native source crop')
    owned = np.array(Image.open(donor_dir / 'donor-mask.png').convert('L')) > 0
    tile_rgb = np.array(Image.open(donor_dir / 'tile.png').convert('RGB'))
    allowed = set(map(tuple, source[owned].tolist()))
    require(all(tuple(v) in allowed for v in tile_rgb.reshape(-1, 3).tolist()),
            'Synthesis contains pixels outside the source-owned donor palette')
    acquire()
    try:
        manifest, scene, names, _, preflight_report = preflight(experiment)
        crowns = {name for name in names if any(m and m.get('foliage_physical_opacity')
                  for m in scene.objects[name].data.materials)}
        require(crowns, 'No physical foliage receivers')
        before = snapshot(scene, crowns)  # Bark is now protected outside appearance.
        image = bpy.data.images.load(str(donor_dir / 'tile.png'), check_existing=False)
        pixels = np.empty(len(image.pixels), np.float32)
        image.pixels.foreach_get(pixels)
        colors = pixels.reshape(image.size[1], image.size[0], 4)
        height, width = colors.shape[:2]
        sin, cos = math.sin(math.radians(35)), math.cos(math.radians(35))

        def sample(obj, normal, positions, known, result, **kwargs):
            # Use a nondegenerate face plane: source-camera projection collapses
            # side-facing surfaces into repeated stripes.
            axis = int(np.argmax(np.abs(np.asarray(normal))))
            scaled = positions * [1., sin, cos]
            axes = [(1, 2), (0, 2), (0, 1)][axis]
            x = np.floor(scaled[:, axes[0]]).astype(int) % width
            y = np.floor(scaled[:, axes[1]]).astype(int) % height
            result[:, :3] = colors[height - 1 - y, x, :3]
            return np.ones(len(positions), bool)

        report = fill([scene.objects[name] for name in sorted(crowns)], sample, None,
                      sha(donor_dir / 'tile.png'), sample_grid=4)
        require(snapshot(scene, crowns) == before, 'Native fill changed protected scene state')
        require(any(row['generated'] for row in report), 'No unknown physical texels filled')
        output.mkdir(parents=True)
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.transparent_max_bounces = 256
        model = output / 'worker.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model), compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.data.scenes[manifest['scene_name']]
        require(snapshot(scene, crowns) == before, 'Reopened native fill changed protected state')
        require(all(sha(Path(p)) == h for p, h in evidence.items()), 'Donor evidence changed')
        render(experiment / 'views.json', output / 'actual', width=manifest['tile_size'][0])
        buffers = []
        for i in range(8):
            im = bpy.data.images.load(str(output / 'actual' / f'view-{i}-textured.png'), check_existing=False)
            data = array('f', [0]) * len(im.pixels)
            im.pixels.foreach_get(data)
            buffers.append(data)
            bpy.data.images.remove(im)
        _tile(buffers, *manifest['tile_size'], output / 'actual' / 'textured.png')
        result = dict(status='PASS', asset_id=manifest['asset_id'], geometry_unchanged=True,
                      known_foliage_rgba_unchanged=True, physical_alpha_unchanged=True,
                      uv_ownership_unchanged=True, bark_and_foreign_appearance_unchanged=True,
                      reopened_preservation='PASS', model_sha256=sha(model),
                      evidence_sha256=evidence, physical_foliage=report,
                      donor_method='texture-synthesis 0.8.3, masked same-asset native donor, seed40',
                      sampling='Native pixel scale in dominant face tangent plane',
                      transparent_bounces=256, approval='pending actual eight-view review',
                      preflight=preflight_report)
        (output / 'validation.json').write_text(json.dumps(result, indent=2) + '\n')
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiment', type=Path)
    parser.add_argument('donor_dir', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.experiment.resolve(), args.donor_dir.resolve(), args.output.resolve())
