"""Isolate unknown off-map foliage faces without changing approved appearance."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement03-refinement/restart2'


def partition(flags):
    known, unknown = [], []
    for face, values in flags.items():
        if not values or any(value not in (0., 1.) for value in values) or len(set(values)) != 1:
            raise ValueError('Cannot partition mixed ownership within one face')
        (known if values[0] == 1 else unknown).append(face)
    assert set(known).isdisjoint(unknown) and set(known) | set(unknown) == set(flags)
    return known, unknown


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def main():
    import bpy
    sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
    from render_slots import acquire, release
    from refinement_workspace import _geometry
    from workspace_components import appearance_state
    root = OUT / 'texture-batch-v7/croisement03-tree-25/experiment'
    source = root / 'approved-model.blend'
    output = root / 'material-partition-v1'
    assert sha(source) == 'd53bf850b4ac24e6f5251de508508527f7bf2670921b7da564ad61ea29c1eb6d'
    assert not output.exists()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source))
        bpy.context.preferences.filepaths.save_version = 0
        bpy.context.window.scene = bpy.data.scenes['Croisement03 Refinement']
        bpy.context.view_layer.update()
        objects = list(bpy.context.scene.objects)
        before_geometry = {obj.name: digest(_geometry(obj)) for obj in objects}
        selected = [obj for obj in objects if obj.type == 'MESH' and obj.get('asset_group') == 'croisement03-tree-25']
        assert len(selected) == 1
        obj = selected[0]
        outside = {o.name: digest(appearance_state(o)) for o in objects if o.type == 'MESH' and o != obj}
        mesh = obj.data
        ownership = mesh.color_attributes['Source ownership']
        before_uv = digest({layer.name: [list(entry.uv) for entry in layer.data] for layer in mesh.uv_layers})
        before_flags = digest([list(entry.color) for entry in ownership.data])
        before_slots = [face.material_index for face in mesh.polygons]
        original_materials = list(mesh.materials)
        before_images = {image.name: hashlib.sha256(image.packed_file.data).hexdigest()
                         for image in bpy.data.images if image.packed_file}
        mappings = []
        for slot, material in enumerate(original_materials):
            if not material or not material.get('foliage_physical_opacity'):
                continue
            faces = [face for face in mesh.polygons if face.material_index == slot]
            flags = {face.index: [ownership.data[i].color[0] for i in face.loop_indices] for face in faces}
            known, unknown = partition(flags)
            if not known or not unknown:
                continue
            assert material.get('foliage_mixed_source_ownership') is True
            clone = material.copy()
            clone.name = material.name + ' / isolated unknown faces'
            new_slot = len(mesh.materials)
            mesh.materials.append(clone)
            for face in unknown:
                mesh.polygons[face].material_index = new_slot
                mesh.attributes['reprojection_fallback_material'].data[face].value = new_slot
            mappings.append(dict(original_slot=slot, unknown_slot=new_slot,
                                 original_material=material.name, unknown_material=clone.name,
                                 protected_faces=known, editable_faces=unknown))
        assert len(mappings) == 1, 'Expected exactly the approved off-map lobe mixed material'
        assert before_geometry == {o.name: digest(_geometry(o)) for o in objects}
        assert outside == {o.name: digest(appearance_state(o)) for o in objects if o.type == 'MESH' and o != obj}
        assert before_uv == digest({layer.name: [list(entry.uv) for entry in layer.data] for layer in mesh.uv_layers})
        assert before_flags == digest([list(entry.color) for entry in ownership.data])
        assert before_images == {image.name: hashlib.sha256(image.packed_file.data).hexdigest()
                                 for image in bpy.data.images if image.packed_file}
        changed = {face.index for face in mesh.polygons if face.material_index != before_slots[face.index]}
        assert changed == set(mappings[0]['editable_faces'])
        for face in mesh.polygons:
            if face.index not in changed:
                assert mesh.materials[face.material_index] == original_materials[before_slots[face.index]]
        # Copying a node material retains each shader input, link and image pointer.
        a = original_materials[mappings[0]['original_slot']]
        b = mesh.materials[mappings[0]['unknown_slot']]
        assert [(node.type, node.name, getattr(node, 'image', None)) for node in a.node_tree.nodes] == [
            (node.type, node.name, getattr(node, 'image', None)) for node in b.node_tree.nodes]
        assert [(link.from_node.name, link.from_socket.name, link.to_node.name, link.to_socket.name) for link in a.node_tree.links] == [
            (link.from_node.name, link.from_socket.name, link.to_node.name, link.to_socket.name) for link in b.node_tree.links]
        output.mkdir()
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'))
        report = dict(status='PASS material-slot partition only; generated appearance remains pending',
                      source_model_sha256=sha(source), output_model_sha256=sha(output / 'model.blend'),
                      geometry_unchanged=True, uv_unchanged=True, ownership_unchanged=True,
                      packed_rgba_unchanged=True, protected_face_materials_unchanged=True,
                      outside_objects_unchanged=len(outside), changed_face_union_exact=True,
                      mappings=mappings, source_uv_sha256=before_uv, source_flags_sha256=before_flags,
                      image_sha256=before_images, recipe_sha256=sha(__file__),
                      rationale='Existing protected and unknown faces shared one atlas. The isolated slot lets the unchanged foliage guard fill an independent image without touching protected faces.')
        (output / 'normalization.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({'status': 'PASS', 'editable_faces': len(changed), 'output': str(output)}))
    finally:
        release()


if __name__ == '__main__':
    main()
