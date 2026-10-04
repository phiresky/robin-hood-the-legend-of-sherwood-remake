"""Export immutable physical-alpha triangles for conservative native occlusion proof."""
import argparse
import hashlib
import io
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from evidence_io import sha, write_json
from tree_geometry import SIN, COS, RAY


def main(worker, destination):
    if destination.exists():
        raise ValueError('Use a fresh immutable export directory')
    model_hash = sha(worker / 'model.blend')
    cfg = json.loads((worker / 'workspace.json').read_text())
    bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
    scene = bpy.data.scenes[cfg['scene_name']]
    bpy.context.window.scene = scene
    crowns = [o for o in bpy.data.collections[cfg['collection_name']].all_objects
              if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id'] and o.get('projection_component') == 'crown']
    if len(crowns) != 1:
        raise ValueError('Expected exactly one crown')
    obj = crowns[0]
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = evaluated.to_mesh()
    arrays, images, records = {}, {}, []
    try:
        mesh.calc_loop_triangles()
        ownership = mesh.color_attributes['Source ownership']
        for triangle in mesh.loop_triangles:
            slot = triangle.material_index
            if slot not in (0, 5):
                continue
            material = mesh.materials[slot]
            if not material.get('foliage_observed') or not all(ownership.data[i].color[0] == 1 for i in triangle.loops):
                raise ValueError('Both proof domains must currently have explicit observed ownership')
            textures = [n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
            if len(textures) != 1 or textures[0].interpolation != 'Closest':
                raise ValueError('Exact proof supports nearest-sampled cutout atlases only')
            texture = textures[0]
            shaders = [n for n in material.node_tree.nodes if n.type == 'BSDF_PRINCIPLED']
            if len(shaders) != 1 or len(shaders[0].inputs['Alpha'].links) != 1:
                raise ValueError('Expected one directly textured physical alpha shader')
            alpha_link = shaders[0].inputs['Alpha'].links[0]
            if alpha_link.from_node != texture or alpha_link.from_socket.name != 'Alpha':
                raise ValueError('Physical alpha must come directly from the exported texture')
            vectors = texture.inputs['Vector'].links
            if len(vectors) != 1 or vectors[0].from_node.type != 'UVMAP':
                raise ValueError('Proof requires explicit untransformed physical UVs')
            uvname = texture.inputs['Vector'].links[0].from_node.uv_map
            layer, image = mesh.uv_layers[uvname], texture.image
            if image.name not in images:
                if image.is_dirty or not image.packed_file:
                    raise ValueError('Expected an immutable packed image')
                packed = image.packed_file.data
                decoded = Image.open(io.BytesIO(packed))
                if decoded.mode != 'RGBA' or packed[:8] != b'\x89PNG\r\n\x1a\n' or packed[24] != 8:
                    raise ValueError('Proof requires an 8-bit RGBA PNG atlas')
                key = f'alpha_{len(images)}'
                alpha = np.asarray(decoded)[:, :, 3][::-1]
                if np.any((alpha != 0) & (alpha != 255)):
                    raise ValueError('Continuous occlusion proof requires binary physical alpha')
                arrays[key] = alpha == 255
                images[image.name] = dict(alpha_key=key, size=list(image.size), packed_sha256=hashlib.sha256(packed).hexdigest(),
                                         extension=texture.extension, interpolation=texture.interpolation, binary_alpha=True)
            records.append(dict(polygon=int(triangle.polygon_index), slot=int(slot), image=image.name,
                                points=[list(obj.matrix_world @ mesh.vertices[i].co) for i in triangle.vertices],
                                uv=[list(layer.data[i].uv) for i in triangle.loops]))
    finally:
        evaluated.to_mesh_clear()
    if not records or not any(r['slot'] == 5 for r in records):
        raise ValueError('No projected interior triangles')
    destination.mkdir(parents=True)
    np.savez_compressed(destination / 'physical-alpha.npz', **arrays)
    if sha(worker / 'model.blend') != model_hash:
        raise RuntimeError('Export changed its worker')
    write_json(destination / 'input.json', dict(worker=str(worker), model_sha256=model_hash, object=obj.name,
        ray=list(RAY), sin=SIN, cos=COS, native_viewport=[0, 0, 1792, 1152], images=images, triangles=records,
        alpha_sha256=sha(destination / 'physical-alpha.npz'), worker_unchanged=True,
        scope='Read-only exact triangle/nearest-alpha input; no visibility or ownership decision yet'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', type=Path)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.worker.resolve(), args.destination.resolve())
