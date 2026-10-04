"""Private RGB experiment on continuously proven hidden native-front faces.

No sampled visibility result authorizes this change. The proof binds the saved
worker and exact nearest-filter physical alpha. Surface placement and alpha are
immutable; uncertain or partly visible faces retain their source material.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from approved_texture_stage import geometry, appearance
from evidence_io import sha, write_json
from render_slots import acquire, release
from render_tree import render_workspace


def main(proof_path, output):
    proof = json.loads(proof_path.read_text())
    source = Path(proof['worker'])
    if output.exists() or sha(source / 'model.blend') != proof['model_sha256']:
        raise ValueError('Use a fresh destination and the exact proven worker')
    if proof['occluder_slots'] != [0, 5] or not proof['worker_unchanged']:
        raise ValueError('Expected unchanged full physical-front occlusion proof')
    input_path = Path(proof['input'])
    if sha(input_path) != proof['input_sha256']:
        raise ValueError('Continuous proof input changed')
    payload = json.loads(input_path.read_text())
    selected = set(proof['hidden_polygons'])
    proven = {r['polygon'] for r in proof['results'] if r['hidden'] and r['uncovered_area'] == 0}
    if selected != proven or not selected:
        raise ValueError('No complete face-level hidden proof')
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        crown = bpy.data.objects[payload['object']]
        mesh = crown.data
        if any(mesh.polygons[i].material_index != 5 for i in selected):
            raise ValueError('Hidden-face indices do not match projected-front material')
        objects = [o for o in bpy.context.scene.objects if o.type == 'MESH']
        before_geometry = {o.name: geometry(o) for o in objects}
        foreign = {o.name: appearance(o) for o in objects if o != crown}
        before_uv = {layer.name: [tuple(row.uv) for row in layer.data] for layer in mesh.uv_layers}
        ownership = mesh.color_attributes['Source ownership']
        before_ownership = [tuple(row.color) for row in ownership.data]
        before_slots = [face.material_index for face in mesh.polygons]
        source_material = mesh.materials[5]
        texture = [n for n in source_material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
        if len(texture) != 1 or texture[0].interpolation != 'Closest':
            raise ValueError('Physical projected-front material differs from the proof')
        packet = source / 'inspection/source-packet'
        front_path = packet / 'observed-interior-front-atlas.png'
        rear_path = packet / 'inferred-interior-atlas.png'
        image_record = payload['images'][texture[0].image.name]
        if sha(front_path) != image_record['packed_sha256']:
            raise ValueError('Native-front atlas differs from the proof')
        packed_front = texture[0].image.packed_file
        rear_textures = [n for n in mesh.materials[4].node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
        if (packed_front is None or hashlib.sha256(bytes(packed_front.data)).hexdigest() != sha(front_path)
                or len(rear_textures) != 1 or rear_textures[0].image.packed_file is None
                or hashlib.sha256(bytes(rear_textures[0].image.packed_file.data)).hexdigest() != sha(rear_path)):
            raise ValueError('Saved physical front or native-derived rear atlas differs from disk')
        front = np.asarray(Image.open(front_path).convert('RGBA')).copy()
        rear = np.asarray(Image.open(rear_path).convert('RGBA'))
        if front.shape[1] != rear.shape[1] * 3 or front.shape[0] % 3:
            raise ValueError('Unexpected projected-front atlas layout')
        # Both atlases were authored from the same card-local UV layout. The
        # front atlas is a nearest 3x expansion with a trimmed bottom; retain
        # every original front alpha texel while using its card-local rear RGB.
        inferred = np.repeat(np.repeat(rear[:front.shape[0] // 3, :, :3], 3, axis=0), 3, axis=1)
        replacement = front.copy()
        replacement[:, :, :3] = inferred
        if not np.array_equal(front[:, :, 3], replacement[:, :, 3]):
            raise ValueError('Physical alpha changed')
        output.mkdir(parents=True)
        inspection = output / 'inspection'
        inspection.mkdir()
        image_path = inspection / 'proven-hidden-card-rgb.png'
        Image.fromarray(replacement).save(image_path)
        material = source_material.copy()
        material.name = crown.name + ' continuously occluded inferred front RGB'
        material['foliage_observed'] = False
        material['texture_provenance'] = 'Own native card-local leaf RGB on continuously proven hidden faces; original physical alpha unchanged'
        material['continuous_occlusion_proof_sha256'] = sha(proof_path)
        new_image = bpy.data.images.load(str(image_path), check_existing=False)
        new_image.pack()
        next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image).image = new_image
        new_slot = len(mesh.materials)
        mesh.materials.append(material)
        changed_loops = set()
        for index in selected:
            face = mesh.polygons[index]
            face.material_index = new_slot
            for loop in face.loop_indices:
                color = list(ownership.data[loop].color)
                color[0] = 0
                ownership.data[loop].color = color
                changed_loops.add(loop)
        if before_geometry != {o.name: geometry(o) for o in objects}:
            raise ValueError('Geometry changed')
        if foreign != {o.name: appearance(o) for o in objects if o != crown}:
            raise ValueError('Foreign appearance changed')
        if before_uv != {layer.name: [tuple(row.uv) for row in layer.data] for layer in mesh.uv_layers}:
            raise ValueError('Foliage UV changed')
        if any(tuple(row.color) != before_ownership[i] for i, row in enumerate(ownership.data) if i not in changed_loops):
            raise ValueError('Source ownership changed outside proven hidden faces')
        if any(face.material_index != before_slots[face.index] for face in mesh.polygons if face.index not in selected):
            raise ValueError('Source material changed outside proven hidden faces')
        # This is a private actual-material comparison, not a ready workspace.
        # Reuse frozen cameras and source-coverage configuration by reference.
        for relative in ['workspace.json', 'source-masks.json', 'modified/views.json',
                         'inspection/refinement.json', 'inspection/source-coverage/report.json']:
            target = output / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / relative, target)
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'), compress=True)
        write_json(inspection / 'hidden-rgb-preservation.json', dict(
            source_worker=str(source), source_model_sha256=proof['model_sha256'],
            model_sha256=sha(output / 'model.blend'), continuous_proof=str(proof_path),
            continuous_proof_sha256=sha(proof_path), changed_faces=sorted(selected),
            geometry_unchanged=True, uv_unchanged=True, physical_alpha_unchanged=True,
            foreign_appearance_unchanged=True, source_visible_materials_unchanged=True,
            ownership_change='Only whole faces proven continuously hidden in the native viewport become inferred',
            source_front_sha256=sha(front_path), native_rear_rgb_sha256=sha(rear_path),
            approval='Private derivative; native rendered comparison and eight-view review pending'))
        render_workspace(output, 384, release_slot=False, transparent_bounces=256)
        if sha(source / 'model.blend') != proof['model_sha256']:
            raise ValueError('Original worker changed')
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('proof', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.proof.resolve(), args.output.resolve())
