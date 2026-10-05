"""Private source-ray branch support trial, retaining native projection and wood."""
import argparse
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'),
                str(ROOT/'level-editor/refinement/blender')]
from approved_texture_stage import geometry, appearance, require
from evidence_io import sha, write_json, record_recipe
from render_slots import acquire, release
from render_tree import render_workspace
from tree_geometry import RAY, SIN, COS


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--anchor', nargs=2, type=float, required=True)
    parser.add_argument('--offset', type=float, required=True)
    parser.add_argument('--radius', type=float, required=True)
    parser.add_argument('--composed-crown-proof', type=Path,
                        help='Original crown envelope when correcting a separately composed wood worker')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    source, output = args.source.resolve(), args.output.resolve()
    require(not output.exists(), 'Fresh support trial required')
    require(args.radius > 0 and math.isfinite(args.offset), 'Invalid support parameters')
    acquire()
    try:
        model_hash = sha(source/'model.blend')
        if args.composed_crown_proof:
            composition_path = source/'inspection/crown-wood-composition.json'
            composition = json.loads(composition_path.read_text())
            require(composition['model_sha256'] == model_hash, 'Stale combined parent')
            envelope_path = args.composed_crown_proof.resolve()
            require(envelope_path == Path(composition['crown_worker'])/'inspection/envelope-preservation.json'
                    and sha(envelope_path) == composition['crown_proof_sha256'], 'Wrong crown envelope')
            proof = json.loads(envelope_path.read_text())
            require(proof['model_sha256'] == composition['crown_model_sha256'], 'Original crown changed')
            proof['parent_combined'] = dict(worker=str(source), model_sha256=model_hash,
                composition_proof=str(composition_path), composition_sha256=sha(composition_path),
                retained_wood_worker=composition['wood_worker'],
                retained_wood_model_sha256=composition['wood_model_sha256'])
        else:
            envelope_path = source/'inspection/envelope-preservation.json'
            proof = json.loads(envelope_path.read_text())
        cfg = json.loads((source/'workspace.json').read_text())
        bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'))
        crown, = [o for o in bpy.data.objects if o.type == 'MESH'
                  and o.get('asset_group') == cfg['asset_id']
                  and o.get('projection_component') == 'crown']
        foreign = {o.name: (geometry(o), appearance(o)) for o in bpy.data.objects
                   if o.type == 'MESH' and o != crown}
        material_before = appearance(crown)
        positions = np.array([crown.matrix_world@v.co for v in crown.data.vertices])
        before = positions.copy()
        parent = list(range(len(positions)))

        def root(i):
            while parent[i] != i:
                parent[i] = parent[parent[i]]
                i = parent[i]
            return i

        for face in crown.data.polygons:
            first = root(face.vertices[0])
            for vertex in face.vertices[1:]:
                parent[root(vertex)] = first
        components = {}
        for i in range(len(positions)):
            components.setdefault(root(i), []).append(i)
        ray = np.array(RAY)
        shifts = []
        for ids in components.values():
            pts = positions[ids]
            center = pts.mean(axis=0)
            screen = np.array([center[0], -center[1]*SIN-center[2]*COS])
            weight = math.exp(-float(np.sum((screen-args.anchor)**2))/(2*args.radius**2))
            shift = args.offset*weight
            # Keep each leaf fragment rigid and above the existing crown floor.
            shift = max(shift, (20-float(pts[:, 2].min()))/SIN)
            positions[ids] += ray*shift
            shifts.append(shift)
        inverse = crown.matrix_world.inverted()
        from mathutils import Vector
        for vertex, position in zip(crown.data.vertices, positions):
            vertex.co = inverse@Vector(position)
        crown.data.update()
        project = lambda p: np.column_stack((p[:, 0], -p[:, 1]*SIN-p[:, 2]*COS))
        drift = float(np.max(np.abs(project(positions)-project(before))))
        require(drift < 1e-3, 'Native projection changed')
        require(appearance(crown) == material_before, 'Crown materials or UVs changed')
        require(foreign == {o.name: (geometry(o), appearance(o)) for o in bpy.data.objects
                           if o.type == 'MESH' and o != crown}, 'Wood or foreign object changed')
        output.mkdir(parents=True)
        (output/'inspection').mkdir()
        shutil.copy2(envelope_path, output/'inspection/prior-crown-envelope.json')
        proof['prior_envelope_sha256'] = sha(envelope_path)
        for name in ['workspace.json', 'source-masks.json', 'modified/views.json',
                     'inspection/refinement.json', 'inspection/source-coverage/report.json']:
            target = output/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source/name, target)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(output/'model.blend'), compress=True)
        proof['model_sha256'] = sha(output/'model.blend')
        proof['non_crown_preservation_reference'] = dict(worker=str(source), model_sha256=model_hash)
        proof['support_reanchor'] = dict(source=str(source), source_model_sha256=model_hash,
            projected_anchor=args.anchor, radius=args.radius, source_ray_offset=args.offset,
            component_shift_range=[min(shifts), max(shifts)], components=len(components),
            maximum_native_projection_drift=drift, original_materials_uvs_and_wood_unchanged=True,
            full_raw_depth_width=float(np.ptp(positions[:, 1])/np.ptp(positions[:, 0])),
            inference='Smooth local depth bend toward existing trunk; not observed depth')
        recipe = record_recipe(output, Path(__file__))
        proof.setdefault('dependency_recipes', {})[str(output/recipe['recipe'])] = recipe['recipe_sha256']
        write_json(output/'inspection/envelope-preservation.json', proof)
        render_workspace(output, 384, release_slot=False, transparent_bounces=256)
        require(sha(source/'model.blend') == model_hash, 'Original trial changed')
    finally:
        release()


if __name__ == '__main__':
    main()
