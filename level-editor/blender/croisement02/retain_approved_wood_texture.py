"""Keep exact approved wood alongside a guarded generated foliage candidate.

This private material selection does not upgrade inferred bark to observed
source. It retains the previous bark when a generated sheet leaves blank wood.
"""
import argparse
from array import array
import json
from pathlib import Path
import sys

import bpy

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
               str(ROOT / 'level-editor/refinement/blender')]
from approved_texture_stage import geometry, appearance
from bake_texture_candidate import snapshot, require
from review_evidence import sha
from refinement_review import _tile
from render_multiview_asset import render
from render_slots import acquire, release


def run(experiment, candidate, output):
    manifest_path = experiment / 'views.json'
    manifest = json.loads(manifest_path.read_text())
    receipt_path = candidate / 'reopened-preservation.json'
    receipt = json.loads(receipt_path.read_text())
    original = experiment / 'approved-model.blend'
    baked = candidate / 'worker.blend'
    require(not output.exists(), 'Use a fresh immutable destination')
    require(receipt['asset_id'] == manifest['asset_id'], 'Candidate asset differs')
    require(receipt['reopened_preservation'] == 'PASS', 'Guarded candidate required')
    require(sha(original) == receipt['model_sha256'], 'Approved model changed')
    require(sha(baked) == receipt['candidate_model_sha256'], 'Candidate changed')
    evidence = {str(p): sha(p) for p in [manifest_path, receipt_path, original, baked]}
    names = set(manifest.get('texture_receiver_object_names', manifest['object_names']))
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(original))
        scene = bpy.data.scenes[manifest['scene_name']]
        bpy.context.window.scene = scene
        meshes = [o for o in scene.objects if o.type == 'MESH']
        baseline_geometry = {o.name: geometry(o) for o in meshes}
        foreign = {o.name: appearance(o) for o in meshes if o.name not in names}
        foliage = {name for name in names if any(m and m.get('foliage_physical_opacity')
                   for m in scene.objects[name].data.materials)}
        wood = names - foliage
        require(foliage and wood, 'Expected separate foliage and wood receivers')
        wood_meshes = {name: scene.objects[name].data.name for name in wood}
        wood_appearance = {name: appearance(scene.objects[name]) for name in wood}
        bpy.ops.wm.open_mainfile(filepath=str(baked))
        scene = bpy.data.scenes[manifest['scene_name']]
        bpy.context.window.scene = scene
        require(baseline_geometry == {o.name: geometry(o) for o in scene.objects
                                     if o.type == 'MESH'}, 'Candidate geometry differs')
        foliage_before = {name: appearance(scene.objects[name]) for name in foliage}
        physical_before = snapshot(scene, names)['physical_foliage']
        with bpy.data.libraries.load(str(original), link=False) as (available, loaded):
            selected = sorted(set(wood_meshes.values()))
            require(set(selected) <= set(available.meshes), 'Approved wood mesh missing')
            loaded.meshes = list(selected)
        imported = dict(zip(selected, loaded.meshes))
        for name, mesh_name in wood_meshes.items():
            scene.objects[name].data = imported[mesh_name]
        bpy.context.view_layer.update()

        def verify(current):
            require(baseline_geometry == {o.name: geometry(o) for o in current.objects
                                         if o.type == 'MESH'}, 'Geometry changed')
            require(foreign == {name: appearance(current.objects[name]) for name in foreign},
                    'Foreign appearance changed')
            require(wood_appearance == {name: appearance(current.objects[name]) for name in wood},
                    'Approved wood appearance or UV changed')
            require(foliage_before == {name: appearance(current.objects[name]) for name in foliage},
                    'Generated foliage appearance changed')
            require(physical_before == snapshot(current, names)['physical_foliage'],
                    'Physical alpha, native RGBA or foliage UV/ownership changed')

        verify(scene)
        output.mkdir(parents=True)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'), compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(output / 'worker.blend'))
        scene = bpy.data.scenes[manifest['scene_name']]
        bpy.context.window.scene = scene
        verify(scene)
        render(manifest_path, output / 'actual', width=manifest['tile_size'][0])
        buffers = []
        width, height = manifest['tile_size']
        for index in range(8):
            image = bpy.data.images.load(str(output / 'actual' / f'view-{index}-textured.png'), check_existing=False)
            try:
                buffer = array('f', [0]) * len(image.pixels)
                image.pixels.foreach_get(buffer)
                buffers.append(buffer)
            finally:
                bpy.data.images.remove(image)
        _tile(buffers, width, height, output / 'actual/textured.png')
        require(evidence == {path: sha(Path(path)) for path in evidence}, 'Evidence changed')
        report = dict(asset_id=manifest['asset_id'], status='PASS',
            original_model_sha256=sha(original), baked_foliage_model_sha256=sha(baked),
            candidate_model_sha256=sha(output / 'worker.blend'), evidence_sha256=evidence,
            wood_objects=sorted(wood), foliage_objects=sorted(foliage),
            geometry_unchanged=True, foreign_appearance_unchanged=True,
            original_wood_appearance_and_uv_unchanged=True, generated_foliage_unchanged=True,
            physical_alpha_native_rgba_and_ownership_unchanged=True, reopened_preservation='PASS',
            bark_provenance='Exact previous approved model data retained; inferred bark stays inferred',
            actual_material_review='pending', texture_approval='pending', publication='not performed')
        (output / 'preservation.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report), flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('experiment', type=Path)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    run(args.experiment.resolve(), args.candidate.resolve(), args.output.resolve())
