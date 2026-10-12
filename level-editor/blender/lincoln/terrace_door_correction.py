"""Correct the covered texels of the terrace roof/wall that show the patch08 door.

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/terrace_door_correction.py -- \
      --worker <combined worker.blend> --output <new directory>

source-states-v2/covered.png omits the closed-door sprite (patch-002 initial) that v1
painted over the unrevealed Patch02 terrace roof. Only the owned-exterior texels of
lincoln-west-tower-terrace building-410/411 change: texels whose surface point is
first-hit visible from the source camera, faces it, and projects into the v1/v2 diff
mask take the v2 RGB. Two texel classes are covered, matching the global reprojection:
texel centres (texel pass) and the bilinear footprint of every covered-state subsample
inside the mask (pixel pass). Geometry, UVs, material graphs, slots, object names and
every other image stay byte-identical; this is checked, and the changed texels are
listed in the report. The worker is opened read-only; <output>/worker.blend is new.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import bpy
import numpy as np

HERE = Path(__file__).resolve().parent
WORK = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
sys.path.insert(0, str(HERE))
import render_slots  # noqa: E402
import global_reproject as G  # noqa: E402
import revealed_state_bake as B  # noqa: E402

ASSET = 'lincoln-west-tower-terrace'
NODES = ('building-410', 'building-411')
TAG = 'lincoln-terrace-door-correction-v1'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(argv)
    output = args.output.resolve()
    if output.exists():
        raise FileExistsError(output)
    render_slots.acquire()
    manifest = json.loads((WORK / 'source-states-v2/manifest.json').read_text())
    for key, path in (('covered_sha256', manifest['covered']), ('v1_diff_mask_sha256', manifest['v1_diff_mask']),
                      ('v1_sha256', manifest['v1'])):
        if sha(path) != manifest[key]:
            raise ValueError('source-states-v2 evidence changed: ' + key)
    from PIL import Image
    v2 = np.asarray(Image.open(manifest['covered']).convert('RGB'))
    diff = np.asarray(Image.open(manifest['v1_diff_mask'])) > 0
    height, width = diff.shape
    worker = args.worker.resolve(strict=True)
    worker_sha = sha(worker)
    bpy.ops.wm.open_mainfile(filepath=str(worker))
    meshes = B.working_meshes()
    # Geometry/UV/slot fingerprints for every mesh; image pixels are digested once per image.
    before = {o.name: B.RG.fingerprint(o) for o in meshes}
    images_before = {i.name: B.image_digest(i) for i in bpy.data.images if i.size[0] and i.has_data}
    targets = [o for o in meshes if o.get('asset_group') == ASSET and o.get('source_node') in NODES
               and not o.hide_render]
    if sorted(o['source_node'] for o in targets) != list(NODES):
        raise ValueError('Expected exactly one visible terrace 410 and 411 mesh')
    visible = [o for o in meshes if not o.hide_render]
    x0, y0, x1, y1 = manifest['changed_bbox']
    zbuffer = B.ZBuffer(visible, (x0 - 8, y0 - 8, x1 + 8, y1 + 8), (width, height))
    report = {'version': 1, 'recipe': TAG, 'worker_in': str(worker), 'worker_in_sha256': worker_sha,
              'source_v2_manifest': str(WORK / 'source-states-v2/manifest.json'),
              'source_v2_sha256': manifest['covered_sha256'], 'objects': {}}
    for obj in targets:
        slot, material, texture, image, uv = B.owned_binding(obj)
        if image.users != 1:
            raise ValueError(f'{obj.name}: owned image is shared; refuse to edit a shared atlas')
        atlas = G.read_image(image)
        new = atlas.copy()
        rec = B.record(obj)
        slot_uv = G.slot_uvs(obj, uv)
        texel_pass = np.zeros(atlas.shape[:2], dtype=bool)
        for face, ty, tx, positions, normals, inner in G.islands(rec, slot_uv, image.size, lambda g: True):
            x, v, _ = G.screen(positions)
            sx, sv = np.floor(x).astype(np.int64), np.floor(v).astype(np.int64)
            take = (normals @ G.TOWARD > 0.05) & (sx >= 0) & (sx < width) & (sv >= 0) & (sv < height)
            take[take] &= diff[sv[take], sx[take]]
            if take.any():
                take[take] &= zbuffer.visible(positions[take])
            new[ty[take], tx[take], :3] = v2[sv[take], sx[take]]
            texel_pass[ty[take], tx[take]] = True
        # Pixel pass: bilinear footprint of first-hit subsamples on this mesh inside the mask.
        flat = zbuffer.ids.ravel()
        cells = np.flatnonzero(flat >= 0)
        tris = flat[cells]
        corners = rec['corners']
        own = {tuple(np.round(c, 6).ravel()): i for i, c in enumerate(corners)}
        hit = np.array([own.get(tuple(np.round(zbuffer.corners[t], 6).ravel()), -1) for t in tris])
        mine = hit >= 0
        cells, local = cells[mine], hit[mine]
        cy, cx = np.divmod(cells, width * G.SS)
        inside = diff[cy // G.SS, cx // G.SS]
        cells, local = cells[inside], local[inside]
        pixel_pass = np.zeros(atlas.shape[:2], dtype=bool)
        if len(cells):
            footprint = G.sample_subsamples(width, rec, local, cells, slot_uv, atlas)
            total = np.zeros(atlas.shape[:2] + (3,))
            count = np.zeros(atlas.shape[:2])
            rgb = v2[(cells // (width * G.SS)) // G.SS, (cells % (width * G.SS)) // G.SS].astype(np.float64)
            for r, c, w in footprint:
                claim = (w > 1e-6) & ~texel_pass[r, c]
                np.add.at(total, (r[claim], c[claim]), rgb[claim])
                np.add.at(count, (r[claim], c[claim]), 1)
            pixel_pass = count > 0
            new[pixel_pass, :3] = np.rint(total[pixel_pass] / count[pixel_pass, None]).astype(np.uint8)
        changed = np.any(new != atlas, axis=2)
        if np.any(new[..., 3] != atlas[..., 3]):
            raise RuntimeError('Alpha changed')
        G.write_image(image, new)
        rows, cols = np.nonzero(changed)
        evidence = output / 'changed-texels' / (hashlib.sha256(obj.name.encode()).hexdigest()[:16] + '.npz')
        evidence.parent.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(evidence, rows=rows, cols=cols, before=atlas[rows, cols], after=new[rows, cols])
        material['terrace_door_correction'] = json.dumps({'recipe': TAG, 'source_v2_sha256': manifest['covered_sha256'],
                                                          'changed_texels': int(changed.sum())}, sort_keys=True)
        report['objects'][obj.name] = {
            'image': image.name, 'atlas_size': list(image.size),
            'texel_pass': int(texel_pass.sum()), 'pixel_pass': int(pixel_pass.sum()),
            'changed_texels': int(changed.sum()), 'mean_abs_change': round(float(
                np.abs(new[changed, :3].astype(int) - atlas[changed, :3]).mean()) if changed.any() else 0.0, 2),
            'evidence': str(evidence), 'evidence_sha256': sha(evidence),
            'image_before_sha256': images_before[image.name], 'image_after_sha256': B.image_digest(image)}
    # Everything except the two corrected images must be byte-identical; geometry/UV/slots too.
    corrected = {row['image'] for row in report['objects'].values()}
    for name, digest in images_before.items():
        if name not in corrected and B.image_digest(bpy.data.images[name]) != digest:
            raise RuntimeError('Unrelated image changed: ' + name)
    for obj in meshes:
        if B.RG.fingerprint(obj) != before[obj.name]:
            raise RuntimeError('Geometry/UV/material slots changed: ' + obj.name)
    report['unchanged'] = {'objects': len(meshes), 'images': len(images_before) - len(corrected)}
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'))
    report['worker_out_sha256'] = sha(output / 'worker.blend')
    (output / 'door-correction.json').write_text(json.dumps(report, indent=2) + '\n')
    print('DOOR-CORRECTION', json.dumps({k: v['changed_texels'] for k, v in report['objects'].items()}),
          report['worker_out_sha256'])


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])
