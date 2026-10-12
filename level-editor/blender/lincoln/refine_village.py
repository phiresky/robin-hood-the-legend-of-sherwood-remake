"""Lincoln village lane geometry recipe (idempotent).

Rebuilds every owned canonical source-node mesh of one village asset from the measured
definitions in ``village_assets.py`` and saves the workspace ``model.blend``. Object
identity, names, transforms, parents and all custom properties (``source_node``,
``asset_group`` ...) are preserved; only mesh data is replaced. Outside assets are
hash-checked before saving. With ``--packet`` the frozen tooling then reprojects and
renders the ``modified/`` packet in the same render-slot lease.

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/refine_village.py -- \
        --asset lincoln-village-northwest-cottage [--packet]
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
ROOT = REPO / 'level-editor/work/needs-review/lincoln-refinement'
TOOLING = ROOT / 'tooling/e6b57cb851c7142b'
sys.path.insert(0, str(HERE))

import bpy  # noqa: E402
import bmesh  # noqa: E402
from mathutils import Vector  # noqa: E402

import village_assets  # noqa: E402

TAG = 'lincoln_village_recipe'
NEUTRAL = 'Lincoln unobserved neutral'


def _geometry_hash(obj):
    payload = {'matrix': [list(r) for r in obj.matrix_world],
               'vertices': [list(v.co) for v in obj.data.vertices] if obj.type == 'MESH' else None,
               'faces': [list(p.vertices) for p in obj.data.polygons] if obj.type == 'MESH' else None}
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


def _mesh(name, obj, shells):
    inverse = obj.matrix_world.inverted()
    verts, faces = [], []
    for label, sv, sf in shells:
        off = len(verts)
        verts.extend(sv)
        faces.extend(tuple(off + i for i in f) for f in sf)
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([inverse @ Vector(p) for p in verts], [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    stats = {'shells': len(shells),
             'nonmanifold_edges': sum(not e.is_manifold for e in bm.edges),
             'boundary_edges': sum(e.is_boundary for e in bm.edges),
             'degenerate_faces': sum(f.calc_area() < 1e-6 for f in bm.faces),
             'nonplanar_faces': 0}
    for f in bm.faces:
        if len(f.verts) > 3:
            n = f.normal
            c = f.calc_center_median()
            if max(abs((v.co - c).dot(n)) for v in f.verts) > 1e-3:
                stats['nonplanar_faces'] += 1
    if stats['nonplanar_faces']:
        # Triangulate only the warped faces so every face is exactly planar.
        warped = [f for f in bm.faces if len(f.verts) > 3 and
                  max(abs((v.co - f.calc_center_median()).dot(f.normal)) for v in f.verts) > 1e-3]
        bmesh.ops.triangulate(bm, faces=warped)
    stats['volume'] = round(bm.calc_volume(signed=True), 2)
    if stats['nonmanifold_edges'] or stats['degenerate_faces'] or stats['volume'] <= 0:
        bm.free()
        raise ValueError(f'{name}: invalid shell {stats}')
    bm.to_mesh(mesh)
    bm.free()
    uv = mesh.uv_layers.new(name='Source projection')
    for loop in mesh.loops:
        p = obj.matrix_world @ mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv = (p.x / 2944, 1 - (-p.y * village_assets.SIN - p.z * village_assets.COS) / 2176)
    material = bpy.data.materials.get(NEUTRAL)
    if material is None:
        material = bpy.data.materials.new(NEUTRAL)
        material.diffuse_color = (.45, .45, .45, 1)
    mesh.materials.append(material)
    return mesh, stats


def refine(asset_id):
    parts, notes = village_assets.build(asset_id)
    owned = [o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == asset_id]
    by_node = {}
    for obj in owned:
        by_node.setdefault(obj.get('source_node'), []).append(obj)
    if set(by_node) != set(parts):
        raise ValueError(f'Recipe nodes {sorted(parts)} differ from owned nodes {sorted(by_node)}')
    report = []
    for node, objs in sorted(by_node.items()):
        if len(objs) != 1:
            raise ValueError(f'{node}: expected one canonical object, found {[o.name for o in objs]}')
        obj = objs[0]
        if obj.modifiers:
            raise ValueError(f'{obj.name}: unexpected modifiers')
        old = obj.data
        mesh, stats = _mesh(obj.name, obj, parts[node])
        obj.data = mesh
        if old.users == 0:
            bpy.data.meshes.remove(old)
        obj[TAG] = 1
        obj['projection_component_labels'] = ', '.join(label for label, _, _ in parts[node])
        obj.hide_render = False
        world = [obj.matrix_world @ v.co for v in mesh.vertices]
        report.append({'object': obj.name, 'source_node': node,
                       'components': [label for label, _, _ in parts[node]],
                       'vertices': len(mesh.vertices), 'faces': len(mesh.polygons),
                       'min_z': round(min(p.z for p in world), 3), 'max_z': round(max(p.z for p in world), 3),
                       **stats})
    return report, notes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--asset', required=True)
    parser.add_argument('--round', default='round-1', help='workspace round directory, e.g. round-2')
    parser.add_argument('--packet', action='store_true', help='also regenerate modified/ with the frozen tooling')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = ROOT / args.round / 'assets' / args.asset
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    outside = {o.name: _geometry_hash(o) for o in bpy.data.objects if o.get('asset_group') != args.asset}
    report, notes = refine(args.asset)
    bpy.context.view_layer.update()
    after = {o.name: _geometry_hash(o) for o in bpy.data.objects if o.get('asset_group') != args.asset}
    if outside != after:
        raise ValueError('Recipe changed an outside object')
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace / 'model.blend'))
    result = {'version': 1, 'asset_id': args.asset, 'recipe': str(Path(__file__).resolve()),
              'objects': report, 'notes': notes, 'outside_objects_preserved': len(outside),
              'ground_z': notes.get('ground_z')}
    (workspace / 'geometry-recipe.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
    if args.packet:
        sys.path.insert(0, str(TOOLING))
        import refinement_workspace
        packet = refinement_workspace.modified(str(workspace))
        print(json.dumps({'packet': packet.get('modified'), 'status': packet.get('status')}))


if __name__ == '__main__':
    main()
