"""Lincoln courtyard lane geometry recipe (idempotent).

Rebuilds every owned mesh of one courtyard-lane asset from the measured shapes
in `courtyard_shapes.py`, standing on the castle plateau (native z 220) instead
of the native datum (z 0). Only the owned meshes' data change; every object
keeps its name, transform, parent and custom properties (source_node,
asset_group, ...). Outside geometry is hashed before and after and must match.

Run from the repository root:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/refine_courtyard.py -- \
        --workspace level-editor/work/needs-review/lincoln-refinement/round-1/assets/<asset> [--packet]

`--packet` regenerates the frozen `modified/` packet afterwards with the
workspace's pinned tooling (projection is stale after any mesh edit).
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import bpy
import bmesh
from mathutils import Matrix, Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, 'level-editor/blender/lincoln')

import courtyard_shapes  # noqa: E402

TAG = 'lincoln_courtyard_recipe'


def _geometry_hash(obj):
    payload = {'matrix': [list(r) for r in obj.matrix_world], 'hide_render': obj.hide_render}
    if obj.type == 'MESH':
        payload['vertices'] = [list(v.co) for v in obj.data.vertices]
        payload['faces'] = [list(p.vertices) for p in obj.data.polygons]
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


def _shell_report(bm):
    return {'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
            'degenerate_faces': sum(f.calc_area() < 1e-6 for f in bm.faces),
            'boundary_edges': sum(e.is_boundary for e in bm.edges)}


def _build_mesh(obj, shells):
    """One mesh per object; each shell is validated closed before merging."""
    inverse = obj.matrix_world.inverted()
    bm_all = bmesh.new()
    reports = []
    for verts, faces in shells:
        bm = bmesh.new()
        bverts = [bm.verts.new(inverse @ Vector([float(c) for c in v])) for v in verts]
        for f in faces:
            bm.faces.new([bverts[i] for i in f])
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        report = _shell_report(bm)
        if any(report.values()):
            bm.free()
            raise ValueError(f'{obj.name}: invalid shell {report}')
        # Positive signed volume confirms outward normals.
        volume = bm.calc_volume(signed=True)
        if volume <= 0:
            bmesh.ops.reverse_faces(bm, faces=list(bm.faces))
            volume = bm.calc_volume(signed=True)
        report['volume'] = round(volume, 3)
        reports.append(report)
        mesh = bpy.data.meshes.new('tmp')
        bm.to_mesh(mesh)
        bm.free()
        bm_all.from_mesh(mesh)
        bpy.data.meshes.remove(mesh)
    old = obj.data
    mesh = bpy.data.meshes.new(old.name)
    bm_all.to_mesh(mesh)
    bm_all.free()
    for material in old.materials:
        mesh.materials.append(material)
    for layer in old.uv_layers:
        mesh.uv_layers.new(name=layer.name)
    if not mesh.uv_layers:
        mesh.uv_layers.new(name='UVMap')
    for polygon in mesh.polygons:
        polygon.material_index = 0
    mesh.update()
    obj.data = mesh
    if old.users == 0:
        bpy.data.meshes.remove(old)
    obj[TAG] = 1
    return reports


def refine(asset_id):
    shapes = courtyard_shapes.build(asset_id)
    owned = {o.get('source_node'): o for o in bpy.context.scene.objects
             if o.type == 'MESH' and o.get('asset_group') == asset_id}
    if set(owned) != set(shapes):
        raise ValueError(f'Owned nodes {sorted(owned)} differ from recipe nodes {sorted(shapes)}')
    result = {}
    for node, obj in sorted(owned.items()):
        if obj.modifiers:
            raise ValueError(f'Unexpected modifiers on {obj.name}')
        shells = _build_mesh(obj, shapes[node])
        world = [obj.matrix_world @ v.co for v in obj.data.vertices]
        native_z = [p.z * courtyard_shapes.C for p in world]
        result[node] = {'object': obj.name, 'shells': shells, 'vertices': len(world),
                        'faces': len(obj.data.polygons),
                        'native_z_range': [round(min(native_z), 2), round(max(native_z), 2)],
                        'geometry_sha256': _geometry_hash(obj)}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', required=True, type=Path)
    parser.add_argument('--packet', action='store_true')
    parser.add_argument('--closeup', action='store_true',
                        help='also render auto-framed eight-view close-ups to inspection/closeup')
    parser.add_argument('--keep-geometry', action='store_true',
                        help='review pass: keep the saved geometry and working masks (e.g. a round-2 '
                             'workspace whose baseline already holds this recipe\'s result)')
    parser.add_argument('--audit', action='store_true',
                        help='write the source-coverage measurement (inspection/source-coverage.json)')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = args.workspace.resolve()
    config = json.loads((workspace / 'workspace.json').read_text())
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    asset_id = config['asset_id']
    before = {o.name: _geometry_hash(o) for o in bpy.data.objects if o.get('asset_group') != asset_id}
    if args.keep_geometry:
        nodes = {o.get('source_node'): {'object': o.name, 'geometry_sha256': _geometry_hash(o),
                                        'faces': len(o.data.polygons), 'kept': True,
                                        'native_z_range': [round(min((o.matrix_world @ v.co).z for v in o.data.vertices) * courtyard_shapes.C, 2),
                                                           round(max((o.matrix_world @ v.co).z for v in o.data.vertices) * courtyard_shapes.C, 2)]}
                 for o in bpy.context.scene.objects if o.type == 'MESH' and o.get('asset_group') == asset_id}
    else:
        nodes = refine(asset_id)
    bpy.context.view_layer.update()
    after = {o.name: _geometry_hash(o) for o in bpy.data.objects if o.get('asset_group') != asset_id}
    if before != after:
        raise ValueError('Recipe changed an outside asset')
    bpy.context.preferences.filepaths.save_version = 0
    if args.keep_geometry:
        mask_report = {'asset_id': asset_id, 'revisions': 'kept (working masks unchanged)'}
    else:
        bpy.ops.wm.save_as_mainfile(filepath=str(workspace / 'model.blend'))
        import courtyard_masks
        mask_report = courtyard_masks.apply(workspace)
    report = {'mask_revisions': mask_report,'version': 1, 'asset_id': asset_id, 'recipe': str(Path(__file__).resolve()),
              'shapes': str((HERE / 'courtyard_shapes.py').resolve()),
              'ground_native_z': courtyard_shapes.GROUND, 'nodes': nodes,
              'outside_objects_preserved': len(before),
              'projection_status': 'stale until the modified packet is regenerated'}
    (workspace / 'geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    if args.packet:
        preparation = json.loads((workspace / 'preparation.json').read_text())
        tooling = Path(preparation['tooling']['directory'])
        sys.path.insert(0, str(tooling))
        import refinement_workspace
        result = refinement_workspace.modified(workspace)
        report['projection_status'] = 'modified packet regenerated'
        report['validation'] = result.get('status')
        (workspace / 'geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    if args.closeup:
        # Supplementary evidence only: the frozen packet cameras are fitted to
        # the old datum pillars, so small props occupy a corner of each view.
        import shutil
        from refinement_review import render_review
        target = workspace / 'inspection' / 'closeup'
        if target.exists():
            shutil.rmtree(target)
        target.parent.mkdir(exist_ok=True)
        render_review(target, scene_name=config['scene_name'], collection_name=config['collection_name'],
                      asset_id=asset_id, source_path=config['source_path'], width=config['width'],
                      height=config['height'], elevation_degrees=config['elevation_degrees'],
                      context_padding=config['context_padding'], lighting=config.get('lighting'),
                      source_mask_manifest=config.get('source_mask_manifest'))
    if args.audit:
        import courtyard_audit
        owned = [o.name for o in bpy.data.objects if o.get('asset_group') == asset_id]

        def first_hit(view, width, height, k, pixels):
            matrix = Matrix(view['camera_matrix_world'])
            right, up = matrix.col[0].xyz.normalized(), matrix.col[1].xyz.normalized()
            direction = -matrix.col[2].xyz.normalized()
            origin0 = matrix.col[3].xyz
            depsgraph = bpy.context.evaluated_depsgraph_get()
            names = []
            for vx, vy in pixels:
                origin = origin0 + right * ((vx + 0.5 - width / 2) / k) - up * ((vy + 0.5 - height / 2) / k)
                hit, _, _, _, obj, _ = bpy.context.scene.ray_cast(depsgraph, origin, direction)
                names.append(obj.name if hit and obj else None)
            return names
        first_hit.owned = owned
        measured = courtyard_audit.audit(workspace, first_hit=first_hit)
        (workspace / 'inspection' / 'source-coverage.json').write_text(json.dumps(measured, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'nodes'}))


if __name__ == '__main__':
    main()
