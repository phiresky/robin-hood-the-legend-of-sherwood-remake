"""Prepare an unapproved inferred-only crown role from exact stored ownership."""
import copy
import hashlib
import json
import struct
from pathlib import Path
import numpy as np
from restart2_static_pair_export_guard import read_glb

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'level-editor/work/croisement03-refinement/restart2'
OUT = BASE / 'approved-tree10-11-inferred-candidate-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    assert not OUT.exists()
    OUT.mkdir()
    rows = []
    for number, kept_count, removed_count in [(10, 6324, 1244), (11, 8080, 1822)]:
        identity = f'croisement03-tree-{number}'
        source_dir = BASE / f'approved-tree10-11-integration-v1/full-export-v1/tree{number}'
        report = json.loads((source_dir / 'report.json').read_text())
        assert sha(source_dir / 'model.glb') == report['model_sha256']
        doc, original_blob = read_glb(source_dir / 'model.glb')
        old_doc = copy.deepcopy(doc)
        blob = bytearray(original_blob)
        def accessor(index):
            a = doc['accessors'][index]; view = doc['bufferViews'][a['bufferView']]
            dtype = np.dtype({5126: '<f4', 5125: '<u4', 5123: '<u2', 5121: 'u1'}[a['componentType']])
            channels = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
            result = np.ndarray((a['count'], channels), dtype=dtype, buffer=original_blob,
                                offset=view.get('byteOffset', 0) + a.get('byteOffset', 0),
                                strides=(view.get('byteStride', dtype.itemsize * channels), dtype.itemsize)).copy()
            if a.get('normalized'): result = result.astype(float) / np.iinfo(dtype).max
            return result
        old_role = 'dynamic-frame0-provenance'
        role = 'inferred-crown-context'
        node_name = f'foliage-{identity}-{old_role}'
        parent_index = next(i for i, n in enumerate(doc['nodes']) if n['name'] == node_name)
        parent = doc['nodes'][parent_index]
        mesh_nodes = [doc['nodes'][i] for i in parent['children']]
        assert len(mesh_nodes) == 1 and 'mesh' in mesh_nodes[0]
        primitives = doc['meshes'][mesh_nodes[0]['mesh']]['primitives']
        kept = removed = 0
        material_ids = set()
        for primitive in primitives:
            assert primitive.get('mode', 4) == 4
            indices = accessor(primitive['indices']).ravel().reshape(-1, 3)
            values = accessor(primitive['attributes']['COLOR_0'])[:, 0][indices]
            assert np.all((values == 0) | (values == 1)) and np.all(values == values[:, :1])
            retained = indices[values[:, 0] == 0].astype('<u4')
            kept += len(retained); removed += int((values[:, 0] == 1).sum())
            while len(blob) % 4: blob.append(0)
            offset = len(blob); data = retained.tobytes(); blob.extend(data)
            view = len(doc['bufferViews'])
            doc['bufferViews'].append({'buffer': 0, 'byteOffset': offset, 'byteLength': len(data), 'target': 34963})
            primitive['indices'] = len(doc['accessors'])
            doc['accessors'].append({'bufferView': view, 'componentType': 5125, 'count': int(retained.size), 'type': 'SCALAR', 'min': [int(retained.min())], 'max': [int(retained.max())]})
            material_ids.add(primitive['material'])
        assert (kept, removed) == (kept_count, removed_count)
        new_name = f'foliage-{identity}-{role}'
        parent['name'] = new_name
        mesh_nodes[0]['name'] = f'Tree{number} inferred crown context only'
        for node in [parent, *mesh_nodes]:
            for key, value in list(node.get('extras', {}).items()):
                if isinstance(value, str): node['extras'][key] = value.replace(node_name, new_name).replace(old_role, role)
        for index in material_ids:
            material = doc['materials'][index]
            assert material['extras']['crown_source_role'] == old_role
            material['extras']['crown_source_role'] = role
        attachment = next(n for n in doc['nodes'] if n['name'] == f'North tree{number}')
        assert attachment['children'].count(parent_index) == 1  # Keep existing crown volume attached.
        assert bytes(blob[:len(original_blob)]) == original_blob
        # New primitive indices are the sole surface selection change. Every original accessor remains exact.
        assert doc['accessors'][:len(old_doc['accessors'])] == old_doc['accessors']
        assert doc['images'] == old_doc['images'] and doc['textures'] == old_doc['textures']
        doc['buffers'][0]['byteLength'] = len(blob)
        encoded = json.dumps(doc, separators=(',', ':')).encode()
        encoded += b' ' * (-len(encoded) % 4)
        blob.extend(b'\0' * (-len(blob) % 4))
        result = struct.pack('<4sII', b'glTF', 2, 28 + len(encoded) + len(blob)) + struct.pack('<II', len(encoded), 0x4e4f534a) + encoded + struct.pack('<II', len(blob), 0x004e4942) + blob
        target = OUT / f'tree{number}'; target.mkdir(); (target / 'model.glb').write_bytes(result)
        descriptor_path = BASE / f'approved-tree10-11-integration-v1/metadata-v1/{identity}/asset-metadata-proposal.json'
        descriptor = json.loads(descriptor_path.read_text())
        original_descriptor = copy.deepcopy(descriptor)
        matched = [part for part in descriptor['parts'] if part['node'] == node_name]
        assert len(matched) == 1 and matched[0]['scenery']
        matched[0]['node'] = new_name
        matched[0]['name'] = 'Inferred crown context; phase guards pending'
        assert [p for p in descriptor['parts'] if p['node'] != new_name] == [p for p in original_descriptor['parts'] if p['node'] != node_name]
        for key in descriptor:
            if key != 'parts': assert descriptor[key] == original_descriptor[key]
        (target / 'asset.json').write_text(json.dumps(descriptor, indent=2) + '\n')
        rows.append({'tree': number, 'source_model': str(source_dir / 'model.glb'), 'source_sha256': sha(source_dir / 'model.glb'),
                     'candidate_model': str(target / 'model.glb'), 'candidate_sha256': sha(target / 'model.glb'),
                     'descriptor_sha256': sha(target / 'asset.json'), 'retained_inferred_triangles': kept,
                     'excluded_observed_frame0_triangles': removed, 'original_binary_prefix_exact': True,
                     'all_original_attribute_texture_bytes_exact': True, 'existing_parts_gameplay_origin_exact': True})
    receipt = {'status': 'HOLD_CPU_CANDIDATE_ONLY', 'rows': rows,
               'scope': 'Retain inferred crown volume and approved bark, excluding observed frame0 proxy faces before proposed coexistence with native Arbre06. Original full exports untouched.',
               'required_before_staging': ['Actual14phase native first-hit and occlusion audit against one complete Arbre06 owner',
                  'Static native pixels and fern/root reserved source guards', 'Saved actual native and oblique comparisons to full approved crowns',
                  'Independent review of changed role semantics and output appearance'],
               'limitations': ['Inferred source ownership is not proof that a face is harmless to native animation rays.',
                  'No animation behavior assigned to inferred volume; no native frame substituted or motion invented.',
                  'These files are not publication-ready or an approved appearance.'], 'canonical_written': False}
    (OUT / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
