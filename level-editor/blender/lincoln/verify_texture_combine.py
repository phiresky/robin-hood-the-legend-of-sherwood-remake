"""Independently verify a texture_combine output against its input worker (read-only).

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/verify_texture_combine.py -- <combine output dir>

Opens the combined worker and links every image of the input worker as a library copy.
Checks: identical object set, mesh geometry, transforms, UV layers, slot assignments and
material node graphs; every image identical except exterior ownership atlases of the combined
assets; in those, every changed texel was neutral gray and opaque in the input (or
unobserved alpha-zero terrain) and is opaque afterwards. The changed-texel total
must not exceed combine.json's generated count. Writes
verification.json into the output directory.
"""
import hashlib
import json
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[3]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def pixels(image):
    width, height = image.size
    data = np.empty(width * height * 4, dtype=np.float32)
    image.pixels.foreach_get(data)
    return np.rint(data.reshape(height, width, 4) * 255).astype(np.uint8)


def editable_texels(data, *, terrain=False):
    """Terrain alpha records ownership; other atlases use opaque neutral gray."""
    if terrain:
        return data[..., 3] == 0
    rgb = data[..., :3]
    return (rgb[..., 0] == rgb[..., 1]) & (rgb[..., 1] == rgb[..., 2]) & (data[..., 3] == 255)


def terrain_atlases(bpy):
    """Identify ownership atlases from the input ground's used material slots."""
    names = set()
    for obj in bpy.data.objects:
        if obj.type != 'MESH' or obj.get('source_node') != 'ground':
            continue
        for slot in {p.material_index for p in obj.data.polygons}:
            material = obj.data.materials[slot]
            if (material and material.use_nodes and material.get('source_ownership_bake')
                    and material.get('source_ownership_label') == 'exterior'
                    and material.get('source_ownership_alpha')):
                names.update(n.image.name for n in material.node_tree.nodes
                             if n.type == 'TEX_IMAGE' and n.image)
    return names


def records(bpy):
    result = {}
    for obj in bpy.data.objects:
        if obj.type != 'MESH':
            continue
        mesh = obj.data
        co = np.empty(len(mesh.vertices) * 3, dtype=np.float32)
        mesh.vertices.foreach_get('co', co)
        faces = np.empty(len(mesh.polygons), dtype=np.int32)
        mesh.polygons.foreach_get('material_index', faces)
        digest = hashlib.sha256(co.tobytes() + faces.tobytes() + np.array(obj.matrix_world, dtype=np.float32).tobytes())
        for layer in mesh.uv_layers:
            uv = np.empty(len(mesh.loops) * 2, dtype=np.float32)
            layer.data.foreach_get('uv', uv)
            digest.update(layer.name.encode() + uv.tobytes())
        for material in mesh.materials:
            digest.update(repr([material.name if material else None,
                                [(n.bl_idname, getattr(getattr(n, 'image', None), 'name', None), getattr(n, 'uv_map', None))
                                 for n in material.node_tree.nodes] if material and material.use_nodes else None]).encode())
        result[obj.name] = digest.hexdigest()
    return result


def main(output):
    import bpy
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from render_slots import acquire
    acquire()
    output = Path(output).resolve()
    combine = json.loads((output / 'combine.json').read_text())
    worker_in = Path(combine['worker_in'])
    if sha(worker_in) != combine['worker_in_sha256'] or sha(output / 'worker.blend') != combine['worker_out_sha256']:
        raise ValueError('Combine input or output worker changed since the combine')
    combined_images = {entry['image'] for asset in combine['assets'] for obj in asset['objects'].values()
                       for entry in obj.get('images', [])}
    # Atlases can be very large: keep only hashes in memory, spill combined atlases to disk.
    spill = output / 'verify-spill'
    spill.mkdir(exist_ok=True)
    bpy.ops.wm.open_mainfile(filepath=str(worker_in))
    before_records = records(bpy)
    ground_images = terrain_atlases(bpy)
    before_images = {}
    for index, image in enumerate(bpy.data.images):
        if not (image.packed_file or image.has_data) or not image.size[0]:
            continue
        data = pixels(image)
        before_images[image.name] = (hashlib.sha256(data.tobytes()).hexdigest(), data.shape, index)
        if image.name in combined_images:
            np.save(spill / f'{index}.npy', data)
        del data
    bpy.ops.wm.open_mainfile(filepath=str(output / 'worker.blend'))
    after_records = records(bpy)
    problems = []
    if before_records != after_records:
        changed = sorted(k for k in set(before_records) | set(after_records) if before_records.get(k) != after_records.get(k))
        problems.append(f'Geometry/UV/slot/material graph changed on {len(changed)} objects: {changed[:10]}')
    changed_texels, changed_images, other_changed = 0, 0, []
    after_names = {image.name for image in bpy.data.images}
    if set(before_images) - after_names:
        problems.append('Images missing after combine: ' + ', '.join(sorted(set(before_images) - after_names))[:500])
    for name, (old_hash, old_shape, index) in before_images.items():
        if name not in after_names:
            continue
        new = pixels(bpy.data.images[name])
        if new.shape != old_shape:
            problems.append('Image size changed: ' + name)
            continue
        if hashlib.sha256(new.tobytes()).hexdigest() == old_hash:
            continue
        if name not in combined_images:
            other_changed.append(name)
            continue
        old = np.load(spill / f'{index}.npy')
        diff = np.any(new != old, axis=2)
        gray = editable_texels(old, terrain=name in ground_images)
        if np.any(diff & ~gray):
            problems.append('Non-neutral texel changed: ' + name)
        if np.any(new[diff][:, 3] != 255):
            problems.append('Changed texel not opaque: ' + name)
        changed_texels += int(diff.sum())
        changed_images += 1
    import shutil
    shutil.rmtree(spill)
    if other_changed:
        problems.append('Images outside combined atlases changed: ' + ', '.join(other_changed[:10]))
    generated = sum(asset['totals']['generated'] for asset in combine['assets'])
    # A generated sample may equal the neutral value exactly; allow only that direction.
    if changed_texels > generated:
        problems.append(f'Changed texels {changed_texels} exceed generated count {generated}')
    report = {'version': 1, 'status': 'FAIL' if problems else 'PASS',
              'worker_in': str(worker_in), 'worker_in_sha256': combine['worker_in_sha256'],
              'worker_out': str(output / 'worker.blend'), 'worker_out_sha256': combine['worker_out_sha256'],
              'objects_compared': len(before_records), 'images_compared': len(before_images),
              'changed_images': changed_images, 'changed_texels': changed_texels,
              'combine_generated_texels': generated, 'problems': problems,
              'method': 'Separate loads of input and output workers; per-object geometry/transform/UV/slot/material-graph hashes; per-image 8-bit pixel comparison.'}
    (output / 'verification.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('status', 'changed_images', 'changed_texels', 'combine_generated_texels', 'problems')}), flush=True)


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1])
