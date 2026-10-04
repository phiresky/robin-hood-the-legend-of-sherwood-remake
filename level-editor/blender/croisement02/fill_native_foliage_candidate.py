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


def main(experiment, donor_dir, output, conditioned=None, conditioned_threshold=.7,
         donor_world_scale=1., inferred_shadow_gamma=1.):
    require(not output.exists(), 'Use a fresh candidate directory')
    require(math.isfinite(conditioned_threshold) and -1 <= conditioned_threshold <= 1,
            'Conditioned projection threshold must be finite and in [-1, 1]')
    require(math.isfinite(donor_world_scale) and donor_world_scale > 0,
            'Donor world scale must be finite and positive')
    require(math.isfinite(inferred_shadow_gamma) and inferred_shadow_gamma > 0,
            'Inferred shadow gamma must be finite and positive')
    evidence = {str(p): sha(p) for p in [donor_dir / 'donor.png', donor_dir / 'donor-mask.png',
                donor_dir / 'tile.png', donor_dir / 'donor-provenance.json', donor_dir / 'donor-validation.json']}
    from PIL import Image
    provenance = json.loads((donor_dir / 'donor-provenance.json').read_text())
    validation = json.loads((donor_dir / 'donor-validation.json').read_text())
    require(validation['status'] == 'PASS', 'Native donor ownership has not passed validation')
    require(sha(Path(provenance['source'])) == provenance['source_sha256'], 'Native source changed')
    require(sha(Path(validation['mask_source'])) == validation['mask_source_sha256'], 'Native mask changed')
    if 'individual_partition' in provenance:
        require(sha(Path(provenance['individual_partition'])) == provenance['individual_partition_sha256'],
                'Individual crown partition changed')
        require(sha(Path(provenance['partition_image'])) == provenance['partition_image_sha256'],
                'Individual crown alpha changed')
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
    conditioned_proof = None
    if conditioned:
        conditioned_proof = json.loads((conditioned / 'source-provenance.json').read_text())
        for key, field in [('source', 'source_sha256'), ('native_mask', 'native_mask_sha256')]:
            require(sha(Path(conditioned_proof[key])) == conditioned_proof[field], 'Continuation source changed')
        canvas = np.array(Image.open(conditioned / 'native-canvas.png').convert('RGB'))
        keep = np.array(Image.open(conditioned / 'known-mask.png').convert('L')) > 0
        generated = np.array(Image.open(conditioned / 'continuation.png').convert('RGB'))
        require(sha(conditioned / 'native-canvas.png') == conditioned_proof['canvas_sha256']
                and sha(conditioned / 'known-mask.png') == conditioned_proof['mask_sha256'],
                'Continuation canvas or mask changed')
        require(np.array_equal(canvas[keep], generated[keep]), 'Inpainting changed native boundary pixels')
        native_palette = set(map(tuple, canvas[keep].tolist()))
        require(all(tuple(v) in native_palette for v in generated.reshape(-1, 3).tolist()),
                'Continuation contains foreign donor pixels')
        evidence.update({str(p): sha(p) for p in conditioned.iterdir() if p.is_file()})
    acquire()
    try:
        manifest, scene, names, _, preflight_report = preflight(experiment)
        if 'approved_model_sha256' in provenance:
            require(sha(experiment / 'approved-model.blend') == provenance['approved_model_sha256'],
                    'Individual crown donor belongs to a different approved model')
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
        continuation = None
        if conditioned:
            image = bpy.data.images.load(str(conditioned / 'continuation.png'), check_existing=False)
            data = np.empty(len(image.pixels), np.float32)
            image.pixels.foreach_get(data)
            continuation = data.reshape(image.size[1], image.size[0], 4)
        conditioned_samples = 0

        def sample(obj, normal, positions, known, result, **kwargs):
            nonlocal conditioned_samples
            # Use a nondegenerate face plane: source-camera projection collapses
            # side-facing surfaces into repeated stripes.
            axis = int(np.argmax(np.abs(np.asarray(normal))))
            scaled = positions * [1., sin, cos] / donor_world_scale
            axes = [(1, 2), (0, 2), (0, 1)][axis]
            x = np.floor(scaled[:, axes[0]]).astype(int) % width
            y = np.floor(scaled[:, axes[1]]).astype(int) % height
            result[:, :3] = colors[height - 1 - y, x, :3]
            facing = float(np.dot(np.asarray(normal), [0., -cos, sin]))
            face = obj.data.polygons[kwargs['face_index']]
            material = obj.data.materials[face.material_index]
            # A double-sided card may present its reverse winding to the source
            # camera. Its projection remains well conditioned at either sign.
            if material.get('foliage_card_sides') != 'paired-one-sided':
                facing = abs(facing)
            if continuation is not None and facing >= conditioned_threshold:
                sx = np.floor(positions[:, 0] - conditioned_proof['origin'][0]).astype(int)
                sy = np.floor(-positions[:, 1] * sin - positions[:, 2] * cos - conditioned_proof['origin'][1]).astype(int)
                take = (sx >= 0) & (sy >= 0) & (sx < continuation.shape[1]) & (sy < continuation.shape[0])
                result[take, :3] = continuation[continuation.shape[0] - 1 - sy[take], sx[take], :3]
                conditioned_samples += int(take.sum())
            if inferred_shadow_gamma != 1.:
                # Only editable inferred texels reach this callback. Preserve
                # chromatic ratios while varying inferred shadow contrast.
                peak = result[:, :3].max(axis=1)
                result[:, :3] *= np.power(np.maximum(peak, 1e-8), inferred_shadow_gamma - 1.)[:, None]
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
                      donor_world_scale=donor_world_scale,
                      inferred_shadow_gamma=inferred_shadow_gamma,
                      conditioned_front_samples=conditioned_samples,
                      conditioned_front_threshold=conditioned_threshold if conditioned else None,
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
    parser.add_argument('--conditioned', type=Path)
    parser.add_argument('--conditioned-threshold', type=float, default=.7,
                        help='Diagnostic projection-normal threshold; default preserves prior candidates')
    parser.add_argument('--donor-world-scale', type=float, default=1.,
                        help='World-space texture wavelength multiplier; does not change source RGB or geometry')
    parser.add_argument('--inferred-shadow-gamma', type=float, default=1.,
                        help='Private inferred-tone diagnostic; protected native texels are excluded')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.experiment.resolve(), args.donor_dir.resolve(), args.output.resolve(),
         args.conditioned.resolve() if args.conditioned else None, args.conditioned_threshold,
         args.donor_world_scale, args.inferred_shadow_gamma)
