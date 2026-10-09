"""Check private York exports against approved surfaces, UVs, and atlas pixels."""
import hashlib
import io
import json
import shutil
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector
from PIL import Image
from scipy.spatial import cKDTree

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
import lossy_assets as la
from render_slots import acquire, release

BASE = ROOT / 'level-editor/work/york-refinement/restart2'
CONFIG = {
    'well': ('restart42-approved-well-export-v2', 'restart38-well-texture-baked-v1'),
    'storehouse': ('restart42-approved-storehouse-export-v1', 'restart38-storehouse-texture-baked-v3'),
}
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()


def image_bytes(doc, buffers, texture):
    image = doc['images'][doc['textures'][texture['index']]['source']]
    view = doc['bufferViews'][image['bufferView']]
    offset = view.get('byteOffset', 0)
    return buffers[view.get('buffer', 0)][offset:offset + view['byteLength']]


def main(key):
    out, source_dir = (BASE / p for p in CONFIG[key])
    proof = json.loads((out / 'export-report.json').read_text())
    source = source_dir / 'model.blend'
    assert sha(source) == proof['approved_model_sha256']
    glb = out / '3d-assets' / proof['asset_id'] / 'model.glb'
    assert sha(glb) == proof['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(source))
    bpy.context.view_layer.update()
    objects = {o.name: o for o in bpy.context.scene.objects if o.type == 'MESH'}
    doc, buffers, _ = la.read_glb(glb)
    nodes = [n for n in doc['nodes'] if 'mesh' in n]
    assert set(objects) == {n['name'] for n in nodes}
    pivot = Vector(proof['source_origin_scene'])
    rows = []
    for node in nodes:
        obj = objects[node['name']]
        assert not any(k in node for k in ('matrix', 'translation', 'rotation', 'scale'))
        mesh = bpy.data.meshes.new_from_object(obj.evaluated_get(bpy.context.evaluated_depsgraph_get()), preserve_all_data_layers=True, depsgraph=bpy.context.evaluated_depsgraph_get())
        mesh.transform(Matrix.Translation(-pivot) @ obj.matrix_world)
        mesh.calc_loop_triangles()
        world = np.array([tuple(v.co) for v in mesh.vertices])
        mesh_doc = doc['meshes'][node['mesh']]
        count = 0
        material_rows = []
        for prim in mesh_doc['primitives']:
            mat = doc['materials'][prim['material']]
            matches = [i for i, m in enumerate(mesh.materials) if m and m.name == mat['name']]
            assert len(matches) == 1, (obj.name, mat['name'], matches)
            slot = matches[0]
            source_mat = mesh.materials[slot]
            image_nodes = [n for n in source_mat.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
            assert len(image_nodes) == 1, (source_mat.name, len(image_nodes))
            image_node = image_nodes[0]
            assert image_node.image.packed_file
            source_rgba = np.array(Image.open(io.BytesIO(image_node.image.packed_file.data)).convert('RGBA'))
            tex = mat['pbrMetallicRoughness']['baseColorTexture']
            exported_rgba = np.array(Image.open(io.BytesIO(image_bytes(doc, buffers, tex))).convert('RGBA'))
            assert np.array_equal(source_rgba, exported_rgba), ('Atlas RGBA changed', obj.name)
            assert mat.get('alphaMode', 'OPAQUE') == 'OPAQUE'
            assert 'KHR_materials_unlit' in mat['extensions']
            assert mat.get('doubleSided', False) == (not source_mat.use_backface_culling)
            vector_links = image_node.inputs['Vector'].links
            assert len(vector_links) == 1 and vector_links[0].from_node.type == 'UVMAP'
            uv_name = vector_links[0].from_node.uv_map
            uv = np.array([tuple(l.uv) for l in mesh.uv_layers[uv_name].data])
            tris = [t for t in mesh.loop_triangles if t.material_index == slot]
            src_pos = np.array([world[list(t.vertices)] for t in tris])
            src_uv = np.array([uv[list(t.loops)] for t in tris])
            pos = la.accessor_array(doc, buffers, prim['attributes']['POSITION'])
            out_uv = la.accessor_array(doc, buffers, prim['attributes'][f"TEXCOORD_{tex.get('texCoord', 0)}"]).copy()
            out_uv[:, 1] = 1 - out_uv[:, 1]
            indices = la.accessor_array(doc, buffers, prim['indices']).reshape(-1, 3)
            assert len(indices) == len(tris), (obj.name, len(indices), len(tris))
            # Cyclic rotations preserve winding while allowing exporter vertex starts.
            source_joint = np.concatenate([src_pos, src_uv * 100], axis=2)
            rotated = np.concatenate([np.roll(source_joint, k, axis=1) for k in range(3)])
            query = np.concatenate([pos[indices], out_uv[indices] * 100], axis=2)
            error, matched = cKDTree(rotated.reshape(-1, 15)).query(query.reshape(-1, 15))
            assert float(error.max()) < .002, (obj.name, float(error.max()))
            assert len(set((matched % len(tris)).tolist())) == len(tris), ('Triangle duplication or omission', obj.name)
            count += len(tris)
            material_rows.append(dict(material=mat['name'], triangles=len(tris), uv_layer=uv_name,
                                      maximum_joint_triangle_position_uv_error=float(error.max()),
                                      atlas_rgba_exact=True, rgba_sha256=hashlib.sha256(source_rgba.tobytes()).hexdigest(),
                                      opaque_unlit_and_culling_exact=True))
        assert count == len(mesh.loop_triangles)
        rows.append(dict(object=obj.name, triangles=count, materials=material_rows))
        bpy.data.meshes.remove(mesh)
    assert sha(source) == proof['approved_model_sha256'] and sha(glb) == proof['model_sha256']
    result = dict(status='PASS approved triangles, winding, UVs, RGBA and opaque unlit materials',
                  approved_model_sha256=sha(source), exported_glb_sha256=sha(glb), objects=rows,
                  limitations=['Private export only; production conversion and browser verification remain separate.',
                               'World/pivot float arithmetic is bounded by joint triangle position/UV error <0.002.'])
    (out / 'source-surface-guard.json').write_text(json.dumps(result, indent=2) + '\n')
    print(key, len(rows), sum(r['triangles'] for r in rows), 'PASS', flush=True)


if __name__ == '__main__':
    assert shutil.disk_usage(ROOT).free > 10 * 1024 ** 3
    assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:'))) * 1024 > 6 * 1024 ** 3
    acquire(slots=2)
    try:
        main(sys.argv[sys.argv.index('--') + 1])
    finally:
        release()
