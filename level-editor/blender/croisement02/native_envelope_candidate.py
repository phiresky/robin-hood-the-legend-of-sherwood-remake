"""Private geometric trial: compact native fronts and retain inferred depth behind."""
import argparse
import json
import shutil
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
                str(ROOT / 'level-editor/refinement/blender')]
from approved_texture_stage import appearance, geometry, require
from evidence_io import sha, write_json
from render_slots import acquire, release
from render_tree import render_workspace
from tree_geometry import RAY, SIN, COS


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--native-depth-fraction', type=float, default=.15)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    source, output = args.source.resolve(), args.output.resolve()
    require(not output.exists(), 'Use a fresh candidate destination')
    require(0 < args.native_depth_fraction < 1, 'Native depth fraction must preserve strict order')
    source_hash = sha(source / 'model.blend')
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source / 'model.blend'))
        cfg = json.loads((source / 'workspace.json').read_text())
        crowns = [o for o in bpy.data.collections[cfg['collection_name']].all_objects
                  if o.type == 'MESH' and o.get('asset_group') == cfg['asset_id']
                  and o.get('projection_component') == 'crown']
        require(len(crowns) == 1, 'Expected one crown')
        crown = crowns[0]
        mesh = crown.data
        objects = [o for o in bpy.data.objects if o.type == 'MESH']
        foreign = {o.name: (geometry(o), appearance(o)) for o in objects if o != crown}
        native_appearance = appearance(crown)
        slots = {p.material_index for p in mesh.polygons}
        require(slots == {0, 2, 4, 5, 6}, 'Unexpected trial crown material layout')
        require(all(mesh.materials[i].get('foliage_observed') is True for i in (0, 5)),
                'Native material roles changed')
        points = np.array([crown.matrix_world @ v.co for v in mesh.vertices], dtype=float)
        ray = np.array(RAY, dtype=float)
        ray /= np.linalg.norm(ray)
        depths = points @ ray
        native_vertices = {v for p in mesh.polygons if p.material_index in (0, 5) for v in p.vertices}
        envelope_vertices = {v for p in mesh.polygons if p.material_index in (0, 2, 5) for v in p.vertices}
        inferred_vertices = {v for p in mesh.polygons if p.material_index in (4, 6) for v in p.vertices}
        require(not envelope_vertices & inferred_vertices, 'Shared vertices prevent scoped envelope movement')
        front = float(depths[list(native_vertices)].max())
        changed_depths = depths.copy()
        envelope = sorted(envelope_vertices)
        inferred = sorted(inferred_vertices)
        changed_depths[envelope] = front + args.native_depth_fraction * (depths[envelope] - front)
        nearest_native_back = float(changed_depths[list(native_vertices)].min())
        inferred_shift = min(0., nearest_native_back - 1. - float(depths[inferred].max()))
        changed_depths[inferred] += inferred_shift
        changed = points + (changed_depths - depths)[:, None] * ray
        inverse = crown.matrix_world.inverted()
        for v, point in zip(mesh.vertices, changed):
            v.co = inverse @ Vector(point)
        mesh.update()
        reopened_points = np.array([crown.matrix_world @ v.co for v in mesh.vertices])
        old_projection = np.column_stack((points[:, 0], -points[:, 1]*SIN-points[:, 2]*COS))
        new_projection = np.column_stack((reopened_points[:, 0], -reopened_points[:, 1]*SIN-reopened_points[:, 2]*COS))
        error = float(np.max(np.abs(old_projection-new_projection)))
        require(error < .001, 'Native ray projection changed')
        require(native_appearance == appearance(crown), 'Crown RGBA, UV, ownership or materials changed')
        require(foreign == {o.name: (geometry(o), appearance(o)) for o in objects if o != crown},
                'Foreign or wood receiver changed')
        output.mkdir(parents=True)
        for relative in ['workspace.json', 'source-masks.json', 'modified/views.json',
                         'inspection/refinement.json', 'inspection/source-coverage/report.json']:
            target = output / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / relative, target)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'model.blend'), compress=True)
        report = dict(status='private geometry trial; visual review pending', source_worker=str(source),
            source_model_sha256=source_hash, model_sha256=sha(output / 'model.blend'),
            native_depth_fraction=args.native_depth_fraction, inferred_depth_shift=inferred_shift,
            native_front_slots=[0, 5], native_front_order='positive affine depth map, identical for every native vertex',
            maximum_native_projection_error=error, all_material_rgba_uv_ownership_unchanged=True,
            non_crown_geometry_and_appearance_unchanged=True,
            method='Native leaf fronts compacted along source rays; separate inferred depth placed behind native envelope',
            limitations=['Native front depth and inferred placements are new geometry requiring review.',
                        'No source texel is reclassified by sampled visibility or replaced by generated appearance.',
                        'Source visibility relative to wood and scene neighbors requires independent comparison.'])
        write_json(output / 'inspection/envelope-preservation.json', report)
        render_workspace(output, 384, release_slot=False, transparent_bounces=256)
        require(sha(source / 'model.blend') == source_hash, 'Original candidate changed')
    finally:
        release()


if __name__ == '__main__':
    main()
