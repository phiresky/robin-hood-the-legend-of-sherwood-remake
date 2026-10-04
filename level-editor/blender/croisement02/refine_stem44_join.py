"""Private joined-surface experiment for the approved forked stem.

Voxel union removes the independently capped row-span sweeps. Native source
coverage is measured before any source reprojection; no approved file is edited.
"""
import argparse
import json
from pathlib import Path
import shutil
import sys

import bpy
import bmesh
import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, scenery_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from stage_review_scene import signature
from source_coverage import audit as coverage
from source_projection_bake import bake
from render_multiview_asset import render
from revise_feedback import geometry


def main(output, voxel, smoothing, mesh_file=None, preview_only=False):
    asset = 'croisement02-supplemental-wood-44'
    worker = scenery_workspace(asset)
    decision = [r for r in json.loads((OUT / 'user-feedback.json').read_text())['records'] if r['asset_id'] == asset][-1]
    before_hash = sha(worker / 'model.blend')
    assert decision['decision'] == 'approved' and decision['model_sha256'] == before_hash
    assert not output.exists(), 'Use a new experiment directory'
    output.mkdir(parents=True)
    (output / 'inspection').mkdir()
    for relative in ('workspace.json', 'inspection/refinement.json'):
        shutil.copyfile(worker / relative, output / relative)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
        bpy.context.preferences.filepaths.save_version = 0
        obj = next(o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == asset)
        outside = {o.name: signature(o) for o in bpy.data.objects if o.type == 'MESH' and o != obj}
        original_geometry = signature(obj)
        if mesh_file:
            from tree_geometry import replace_mesh
            supplied = np.load(mesh_file)
            replace_mesh(obj, supplied['vertices'].tolist(), supplied['faces'].tolist(), materials=[])
        bpy.ops.object.select_all(action='DESELECT')
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        modifier = obj.modifiers.new('Union independently capped branch sweeps', 'REMESH')
        modifier.mode = 'VOXEL'
        modifier.voxel_size = voxel
        modifier.use_smooth_shade = True
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        modifier = obj.modifiers.new('Relax row-sweep ridges', 'SMOOTH')
        modifier.factor = .5
        modifier.iterations = smoothing
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        for polygon in obj.data.polygons:
            polygon.use_smooth = True
        topology = bmesh.new()
        topology.from_mesh(obj.data)
        assert all(edge.is_manifold for edge in topology.edges), 'Joined surface is not closed manifold geometry'
        bmesh.ops.recalc_face_normals(topology, faces=list(topology.faces))
        topology.to_mesh(obj.data)
        topology.free()
        obj.data.materials.clear()
        neutral = bpy.data.materials.new('Private joined stem neutral')
        neutral.diffuse_color = (.35, .35, .35, 1)
        neutral.use_nodes = True
        shader = neutral.node_tree.nodes.get('Principled BSDF')
        shader.inputs['Base Color'].default_value = (.35, .35, .35, 1)
        shader.inputs['Emission Color'].default_value = (.12, .12, .12, 1)
        shader.inputs['Emission Strength'].default_value = 1
        obj.data.materials.append(neutral)
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'))
        report = coverage(output, [obj])
        provenance = dict(status='private unapproved geometry experiment', source_worker=str(worker),
            approved_model_sha256=before_hash, approved_geometry_sha256=original_geometry,
            candidate_geometry_sha256=signature(obj), voxel_size=voxel, smoothing_iterations=smoothing,
            mesh_derivation=str(mesh_file) if mesh_file else None,
            mesh_derivation_sha256=sha(mesh_file) if mesh_file else None,
            closed_manifold_surface=True,
            source_coverage=report, original_geometry_approval_not_transferred=True,
            prior_texture_candidate_preserved=True)
        write_json(output / 'join-experiment.json', provenance)
        if report['intersection_over_union'] < .95:
            raise ValueError('Joined candidate misses native silhouette: ' + str(report['intersection_over_union']))
        cfg = json.loads((worker / 'workspace.json').read_text())
        native_geometry = signature(obj)
        projection_geometry = geometry([obj])
        if not preview_only:
            bake('Croisement02', cfg['source_path'], output / 'inspection/source-ownership.json',
                receiver_nodes=[obj['source_node']], occluder_nodes=[obj['source_node']],
                projection_label='exterior', preserve_authored=False,
                source_mask_manifest=cfg['source_mask_manifest'])
        # Reprojection creates UVs, so the post-bake signature is recorded
        # separately; source ownership and geometry are guarded by the baker.
        provenance['before_projection_signature'] = native_geometry
        assert geometry([obj]) == projection_geometry, 'Source reprojection changed joined geometry'
        provenance['source_projection_pending'] = preview_only
        if not preview_only:
            provenance['source_ownership_sha256'] = sha(output / 'inspection/source-ownership.json')
        assert outside == {o.name: signature(o) for o in bpy.data.objects if o.type == 'MESH' and o != obj}
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'))
        provenance['source_coverage'] = coverage(output, [obj])
        assert provenance['source_coverage']['intersection_over_union'] >= .95
        scene = bpy.data.scenes['Croisement02 Refinement']
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        frames = json.loads((worker / 'modified/views.json').read_text())
        frames['source_blend'] = str(output / 'model.blend')
        for view in frames['views']:
            view['crop'] = dict(width=frames['tile_size'][0], height=frames['tile_size'][1])
        write_json(output / 'views.json', frames)
        render(output / 'views.json', output / 'views', modes=('textured', 'solid'), width=256)
        for mode in ('solid', 'textured'):
            width, height = frames['tile_size']
            sheet = Image.new('RGBA', (width * 4, height * 2))
            for i in range(8):
                sheet.paste(Image.open(output / f'views/view-{i}-{mode}.png'), ((i % 4) * width, (i // 4) * height))
            sheet.save(output / f'{mode}.png')
        provenance.update(model_sha256=sha(output / 'model.blend'), outside_geometry_and_uv_unchanged=True,
            solid_sha256=sha(output / 'solid.png'), textured_sha256=sha(output / 'textured.png'))
        assert sha(worker / 'model.blend') == before_hash
        write_json(output / 'join-experiment.json', provenance)
        print(json.dumps(provenance), flush=True)
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--voxel', type=float, default=.35)
    parser.add_argument('--smoothing', type=int, default=12)
    parser.add_argument('--mesh-file', type=Path)
    parser.add_argument('--preview-only', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.output.resolve(), args.voxel, args.smoothing, args.mesh_file, args.preview_only)
