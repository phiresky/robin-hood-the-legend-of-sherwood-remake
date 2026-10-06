"""Compare the same physical crown conversion before and after approved wood fill."""
import json
import struct
import sys
from pathlib import Path

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from evidence_io import sha, write_json
from export_editor import export_editor
from render_slots import acquire, release

B = ROOT / 'level-editor/work/croisement03-refinement/restart2'


def export(path, tree, target):
    bpy.ops.wm.open_mainfile(filepath=str(path))
    scene = bpy.data.scenes.get('Croisement03 Refinement') or bpy.data.scenes.new('Croisement03 Refinement')
    bpy.context.window.scene = scene
    leaf = next(o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == f'croisement03-arbre06-fragment-tree{tree}-provisional')
    working = bpy.data.collections.get('Croisement03 Working') or bpy.data.collections.new('Croisement03 Working')
    if working.name not in scene.collection.children:
        scene.collection.children.link(working)
    if leaf.name not in working.objects:
        working.objects.link(leaf)
    leaf['asset_group'] = f'croisement03-tree-{tree}-crown-comparison'
    leaf['source_node'] = f'foliage-croisement03-tree{tree}-arbre06-provisional'
    leaf['asset_name'] = f'Tree {tree} crown comparison'
    leaf['part_name'] = 'Provisional physical crown; dynamic membership unresolved'
    leaf.hide_render = False
    original = leaf.data.materials[0]
    image = next(n.image for n in original.node_tree.nodes if n.type == 'TEX_IMAGE')
    mat = original.copy()
    mat.node_tree.nodes.clear()
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    output = nodes.new('ShaderNodeOutputMaterial')
    shader = nodes.new('ShaderNodeBsdfPrincipled')
    tex = nodes.new('ShaderNodeTexImage')
    tex.image, tex.interpolation, tex.extension = image, 'Closest', 'CLIP'
    links.new(tex.outputs['Color'], shader.inputs['Base Color'])
    links.new(tex.outputs['Alpha'], shader.inputs['Alpha'])
    links.new(shader.outputs[0], output.inputs[0])
    mat['private_foliage_alpha'] = True
    for key, value in {'foliage_physical_opacity': True, 'opacity_semantics': 'physical-coverage',
                       'source_ownership_semantics': 'separate-mask', 'source_ownership_channel': 'vertex-color-r',
                       'source_ownership_backface': 'inferred', 'foliage_backface_fill': 'source-derived',
                       'foliage_unlit': True}.items():
        mat[key] = value
    leaf.data.materials[0] = mat
    native_faces = 950 if tree == 12 else 434
    ownership = leaf.data.color_attributes.new(name='Source ownership', type='FLOAT_COLOR', domain='CORNER')
    for face in leaf.data.polygons:
        for loop in face.loop_indices:
            ownership.data[loop].color = (1 if face.index < native_faces else 0, 1, 1, 1)
    index = list(leaf.data.color_attributes).index(ownership)
    leaf.data.color_attributes.active_color_index = index
    leaf.data.color_attributes.render_color_index = index
    report = export_editor('Croisement03', target, asset_id=leaf['asset_group'])
    data = target.read_bytes()
    length, kind = struct.unpack_from('<II', data, 12)
    doc = json.loads(data[20:20 + length])
    for material in doc['materials']:
        if material.get('extras', {}).get('private_foliage_alpha'):
            material.update(alphaMode='MASK', alphaCutoff=.5, doubleSided=True)
    encoded = json.dumps(doc, separators=(',', ':')).encode()
    encoded += b' ' * (-len(encoded) % 4)
    tail = data[20 + length:]
    target.write_bytes(struct.pack('<III', 0x46546c67, 2, 20 + len(encoded) + len(tail)) + struct.pack('<II', len(encoded), kind) + encoded + tail)
    return report


def main():
    out = B / 'adjacent-crown-renderer-audit-v1/export-identity-v2'
    assert not out.exists()
    out.mkdir()
    acquire()
    try:
        rows = []
        for tree, revision in ((12, 7), (14, 5)):
            before = B / f'tree{tree}-crownfragment-v{revision}/worker.blend'
            after = B / f'tree{tree}-approved-wood-texture-v1/source-restored-fill-v1/worker.blend'
            before_target, after_target = out / f'tree{tree}-before.glb', out / f'tree{tree}-after.glb'
            a, b = export(before, tree, before_target), export(after, tree, after_target)
            assert sha(before_target) == sha(after_target), (tree, sha(before_target), sha(after_target))
            assert a['placement_origin_scene'] == b['placement_origin_scene']
            rows.append(dict(tree=tree, before_sha256=sha(before_target), after_sha256=sha(after_target),
                             exact_binary_identity=True, pivot=a['placement_origin_scene']))
        write_json(out / 'receipt.json', dict(status='PASS exact same-renderer crown inputs before/after fill', trees=rows,
            scope='The complete isolated crown glTF, including geometry, normals, UV, COLOR0, material/sampler/alpha configuration and embedded textures, is byte-identical before and after wood fill. With identical production TextureDisplay and cameras there is no changed crown renderer input. No new browser render or appearance approval implied.'))
    finally:
        release()


if __name__ == '__main__':
    main()
