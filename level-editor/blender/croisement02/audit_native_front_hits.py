"""Read-only physical-alpha native-camera first-hit faces and atlas texels."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
for folder in ('level-editor/blender/croisement02', 'level-editor/refinement', 'level-editor/refinement/blender'):
    sys.path.insert(0, str(ROOT / folder))
from evidence_io import sha, write_json
from refinement_review import _tree
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY


def main(worker, destination, scale):
    if destination.exists() or not 1 <= scale <= 8:
        raise ValueError('Use a fresh destination and scale 1..8')
    model_hash = sha(worker / 'model.blend')
    cfg = json.loads((worker / 'workspace.json').read_text())
    coverage_path = worker / 'inspection/source-coverage/report.json'
    coverage = json.loads(coverage_path.read_text())
    if coverage['model_sha256'] != model_hash:
        raise ValueError('Source camera report is stale')
    left, top, right, bottom = coverage['source_crop']
    destination.mkdir(parents=True)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
        scene = bpy.data.scenes[cfg['scene_name']]
        bpy.context.window.scene = scene
        objects = [o for o in list(bpy.data.collections[cfg['collection_name']].all_objects)
                   if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id']]
        tree, owners, _ = _tree(objects)
        records, target_triangles, images = [], [], {}
        for obj in objects:
            evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
            mesh = evaluated.to_mesh()
            try:
                mesh.calc_loop_triangles()
                for triangle in mesh.loop_triangles:
                    material = mesh.materials[triangle.material_index]
                    target = obj.get('projection_component') == 'crown' and triangle.material_index == 5
                    record = dict(object=obj.name, polygon=int(triangle.polygon_index), slot=int(triangle.material_index), target=target)
                    if target:
                        if not material.get('foliage_observed') or 'interior projected front' not in material.name:
                            raise ValueError('Slot5 is not the expected protected projected-interior material')
                        textures = [n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
                        if len(textures) != 1 or textures[0].interpolation != 'Closest':
                            raise ValueError('Unsupported target atlas sampling')
                        texture = textures[0]
                        uvname = texture.inputs['Vector'].links[0].from_node.uv_map
                        layer = mesh.uv_layers[uvname]
                        image = texture.image
                        if image.is_dirty or not image.packed_file:
                            raise ValueError('Expected an immutable packed target atlas')
                        record.update(points=np.array([obj.matrix_world @ mesh.vertices[i].co for i in triangle.vertices]),
                                      uvs=np.array([layer.data[i].uv[:] for i in triangle.loops]), image=image.name,
                                      size=tuple(image.size))
                        if image.name not in images:
                            images[image.name] = dict(size=list(image.size), packed_sha256=hashlib.sha256(image.packed_file.data).hexdigest(),
                                                     filepath=image.filepath)
                        target_triangles.append(len(records))
                    records.append(record)
            finally:
                evaluated.to_mesh_clear()
        if len(records) != len(owners) or not target_triangles:
            raise ValueError('Physical BVH records do not match expected target faces')
        width, height = (right - left) * scale, (bottom - top) * scale
        first = np.full((height, width), -1, np.int32)
        atlas_xy = np.full((height, width, 2), -1, np.int32)
        target_hits = np.zeros((height, width), bool)
        for y in range(height):
            sy = top + (y + .5) / scale
            for x in range(width):
                sx = left + (x + .5) / scale
                origin = Vector((sx, -sy * SIN, -sy * COS)) + RAY * 5000
                hit, _, triangle, _ = tree.ray_cast(origin, -RAY)
                if hit is None:
                    continue
                first[y, x] = triangle
                record = records[triangle]
                if not record['target']:
                    continue
                points, uvs = record['points'], record['uvs']
                basis = np.stack([points[1] - points[0], points[2] - points[0]], axis=1)
                weights = np.linalg.lstsq(basis, np.array(hit) - points[0], rcond=None)[0]
                uv = uvs[0] * (1 - weights.sum()) + uvs[1] * weights[0] + uvs[2] * weights[1]
                iw, ih = record['size']
                atlas_xy[y, x] = (int(np.floor(uv[0] * iw)) % iw, int(np.floor(uv[1] * ih)) % ih)
                target_hits[y, x] = True
            if y % 100 == 0:
                print(f'Native first-hit row {y}/{height}', flush=True)
        visible_triangles = sorted(int(i) for i in np.unique(first[target_hits]))
        visible_polygons = sorted({records[i]['polygon'] for i in visible_triangles})
        unique_texels = np.unique(atlas_xy[target_hits], axis=0)
        np.savez_compressed(destination / 'first-hit-samples.npz', first_triangle=first, target_atlas_xy=atlas_xy)
        Image.fromarray(target_hits.astype('uint8') * 255).save(destination / 'projected-interior-first-hit.png')
        Image.fromarray((first >= 0).astype('uint8') * 255).save(destination / 'physical-first-hit.png')
        if sha(worker / 'model.blend') != model_hash:
            raise RuntimeError('Read-only audit changed its source')
        write_json(destination / 'evidence.json', dict(worker=str(worker), model_sha256=model_hash,
            source_camera_report_sha256=sha(coverage_path), native_crop=coverage['source_crop'], scale=scale,
            target_slot=5, target_triangles=len(target_triangles), visible_target_triangles=visible_triangles,
            visible_target_polygons=visible_polygons, first_hit_samples=int((first >= 0).sum()),
            target_first_hit_samples=int(target_hits.sum()), unique_target_texels=int(len(unique_texels)),
            target_fraction=float(target_hits.sum() / (first >= 0).sum()), target_atlases=images,
            target_triangle_records=[dict(index=i, **{k: v.tolist() if isinstance(v, np.ndarray) else v
                                                     for k, v in records[i].items()}) for i in target_triangles],
            worker_unchanged=True, approval='diagnostic only; no ownership changed',
            limitation='Finite subpixel sample lattice; unsampled texels are not automatically unobserved. Any ownership derivative needs conservative protection and independent native appearance verification.',
            file_sha256={p.name: sha(p) for p in destination.iterdir() if p.is_file()}))
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--scale', type=int, default=4)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.worker.resolve(), args.destination.resolve(), args.scale)
