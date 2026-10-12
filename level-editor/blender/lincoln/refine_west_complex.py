"""Lincoln west-complex geometry refinement recipe (lane ``west_complex``).

Idempotent: every owned mesh is rebuilt from native sight-obstacle data plus
the artwork measurements recorded in the per-asset ``west_complex_*`` modules,
so rerunning on an already refined ``model.blend`` produces the same geometry.

Run from the repository root, one asset per Blender process:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/refine_west_complex.py -- \
        --asset lincoln-garden-fountain [--no-packet]

The recipe acquires a Lincoln render slot, opens the asset workspace's
``model.blend``, replaces only meshes whose ``asset_group`` is the asset,
preserves object identity/custom properties/world transforms, saves, and then
regenerates the frozen-camera ``modified/`` packet with the pinned tooling.
"""
import argparse
import hashlib
import importlib
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
REFINEMENT = ROOT / 'work/needs-review/lincoln-refinement'
# Workspace round: round-1 (default) or round-2 (integrated context); set with
# --round or the WEST_COMPLEX_ROUND environment variable.
import os as _os
ROUND = int(_os.environ.get('WEST_COMPLEX_ROUND', '1'))
ASSETS_DIR = REFINEMENT / f'round-{ROUND}/assets'
TOOLING = REFINEMENT / 'tooling/e6b57cb851c7142b'

MODULES = {
    'lincoln-west-round-tower': 'west_complex_towers',
    'lincoln-west-tower-hall': 'west_complex_towers',
    'lincoln-west-slate-tower': 'west_complex_towers',
    'lincoln-west-tower-terrace': 'west_complex_terrace',
    'lincoln-west-upper-curtain-wall': 'west_complex_curtain',
    'lincoln-garden-north-wall': 'west_complex_garden',
    'lincoln-garden': 'west_complex_garden',
    'lincoln-garden-fountain': 'west_complex_garden',
    'lincoln-inner-west-gate': 'west_complex_gate',
}
NEUTRAL = 'Lincoln west complex / unknown source'
SCRATCH = REFINEMENT / 'scratch/west_complex'

# Reviewed working-mask revisions, applied idempotently from each workspace's
# frozen mask-reference/assignments.json before projection. Exclusions record
# foreground occlusion (vegetation, sprites) drawn in front of a receiver.
FOLIAGE_154 = ('Mask 154 is the orange tree standing on the rock below the upper curtain wall; it is painted '
               'in front of the cone-tower shaft, the hall east end and the curtain-wall foot.')
BUSH_65 = ('Mask 65 is the bush growing at the foot of the cone tower and the bastion east wall; it is '
           'painted in front of that masonry.')
BUSH_67 = ('Mask 67 is the bush in front of the inner west gate east block and ramp wall foot.')
DOOR_418 = ('Mask 418 is the Patch08-initial closed-door sprite drawn over the roof paving and parapet; the door '
            '(462) physically stands inside the bastion under the Patch02 roof, so its sprite must not be painted '
            'onto the bastion receivers.')
VEG_A = str(SCRATCH / 'veg-masks-a.png')
TERRACE_VEG = str(SCRATCH / 'terrace-veg.png')
GATE_VEG = str(SCRATCH / 'gate-veg.png')


def _rev(nodes, add=(), remove=(), reason='', evidence=()):
    return {f'building-{n:03}': {'add': list(add), 'remove': list(remove), 'reason': reason,
                                 'evidence': list(evidence)} for n in nodes}


def _merge(*parts):
    out = {}
    for part in parts:
        for node, rev in part.items():
            cur = out.setdefault(node, {'add': [], 'remove': [], 'reason': '', 'evidence': []})
            cur['add'] += rev['add']
            cur['remove'] += rev['remove']
            cur['reason'] = (cur['reason'] + ' ' + rev['reason']).strip()
            cur['evidence'] += rev['evidence']
    return out


CONE_291 = (377, 380, 381, 391, 396, 397)
HALL_291 = (386, 392, 393, 394, 395, 407, 408)
MASK_REVISIONS = {
    'lincoln-west-slate-tower': _merge(_rev(CONE_291, add=[154], reason=FOLIAGE_154, evidence=[VEG_A]),
                                       _rev((377,), add=[65], reason=BUSH_65, evidence=[VEG_A, TERRACE_VEG])),
    'lincoln-west-tower-hall': _rev(HALL_291, add=[154], reason=FOLIAGE_154, evidence=[VEG_A]),
    'lincoln-west-round-tower': _rev((403,), remove=[291], reason=(
        'Chimney 403: its body is drawn inside masks 250 and 291 (inspection/chimney-masks.png: 249 cyan, 250 '
        'magenta, 291 yellow); the round-tower 291 exclusion removed the chimney from its own receiver. First-hit '
        'gating keeps hall-roof pixels on the hall receivers.'),
        evidence=[str(REFINEMENT / 'round-1/assets/lincoln-west-round-tower/inspection/chimney-masks.png')]),
    'lincoln-west-tower-terrace': _merge(
        _rev((60, 69, 410, 411, 412, 413), add=[418], reason=DOOR_418,
             evidence=[str(SCRATCH / 'terrace/door-mask.png'), str(SCRATCH / 'terrace/door-rev.png')]),
        _rev((60, 69, 410, 411, 412, 413), add=[65], reason=BUSH_65, evidence=[TERRACE_VEG])),
    'lincoln-west-upper-curtain-wall': _rev((281, 282, 283, 284, 285), add=[154], reason=FOLIAGE_154,
                                            evidence=[VEG_A]),
    'lincoln-inner-west-gate': _rev(tuple(range(289, 306)), add=[67], reason=BUSH_67, evidence=[GATE_VEG]),
}


# Covered-state display: applied-state nodes are hidden in the exterior packet.
# 463 is the open bastion door that exists only after Patch08 is applied; the
# covered artwork shows the closed door 462 (mask 418). The object and its
# geometry are kept. Hiding it for the covered packet is not possible with the
# pinned tooling: refinement_review rejects a change of projection receiver
# membership against the frozen input unless a reviewed projection manifest
# exists (none for this workspace), so 463 stays render-visible and is tagged.
STATE_VISIBILITY = {'lincoln-west-tower-terrace': {463: {
    'hide_render': False, 'state': 'patch08-applied-only; should be hidden in the covered state '
                                   '(needs a reviewed projection/state manifest)'}}}


def apply_mask_revisions(asset):
    """Rewrite the working source-masks.json from the frozen origin plus reviewed revisions."""
    workspace = ASSETS_DIR / asset
    frozen = json.loads((workspace / 'mask-reference/assignments.json').read_text())
    revisions = MASK_REVISIONS.get(asset, {})
    applied = {}
    for entry in frozen['projections']['exterior']['assignments']:
        rev = revisions.get(entry.get('source_node'))
        if not rev or entry.get('asset_group') is not None:
            continue
        before = list(entry.get('exclude_mask_indices', []))
        after = sorted((set(before) | set(rev['add'])) - set(rev['remove']))
        if after == before:
            continue
        entry['exclude_mask_indices'] = after
        if after:
            entry['exclusions_reviewed'] = True
            entry['exclusion_reason'] = (entry.get('exclusion_reason', '') + ' West-complex review: ' + rev['reason']).strip()
        entry['west_complex_revision'] = {'excludes_before': before, 'excludes_after': after,
                                          'reason': rev['reason'], 'evidence': {
                                              p: sha(p) for p in rev['evidence']}}
        applied[entry['source_node']] = after
    (workspace / 'source-masks.json').write_text(json.dumps(frozen, indent=1) + '\n')
    return applied


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def replace_mesh(obj, world_verts, faces, strict=True):
    import bpy
    import bmesh
    from mathutils import Vector
    inverse = obj.matrix_world.inverted()
    mesh = bpy.data.meshes.new(obj.name + ' / west complex refinement')
    mesh.from_pydata([inverse @ Vector(v) for v in world_verts], [], faces)
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-4)
    bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    nonmanifold = sum(not e.is_manifold for e in bm.edges)
    degenerate = sum(f.calc_area() < 1e-6 for f in bm.faces)
    volume = bm.calc_volume(signed=True)
    bm.to_mesh(mesh)
    bm.free()
    if strict and (nonmanifold or degenerate or volume <= 0):
        raise ValueError(f'Invalid replacement topology {obj.name}: nonmanifold={nonmanifold} '
                         f'degenerate={degenerate} volume={volume}')
    material = bpy.data.materials.get(NEUTRAL)
    if material is None:
        material = bpy.data.materials.new(NEUTRAL)
        material.diffuse_color = (.45, .45, .45, 1)
    mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    old = obj.data
    obj.data = mesh
    if old.users == 0:
        bpy.data.meshes.remove(old)
    obj['west_complex_recipe'] = 'refine_west_complex.py'
    return {'vertices': len(mesh.vertices), 'faces': len(mesh.polygons),
            'nonmanifold_edges': nonmanifold, 'degenerate_faces': degenerate,
            'signed_volume_world': round(volume, 3)}


def apply(asset, packet=True):
    import bpy
    workspace = ASSETS_DIR / asset
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    config = json.loads((workspace / 'workspace.json').read_text())
    module = importlib.import_module(MODULES[asset])
    objects = [o for o in bpy.data.collections[config['collection_name']].all_objects
               if o.type == 'MESH' and o.get('asset_group') == asset]
    if hasattr(module, 'build_owned'):
        shapes, info = module.build_owned(asset, sorted({int(o['source_node'].split('-')[1]) for o in objects}))
    else:
        shapes, info = module.build(asset)
    by_node = {}
    for obj in objects:
        by_node.setdefault(int(obj['source_node'].split('-')[1]), []).append(obj)
    # Nodes regrouped to another asset (catalog v3 moves, e.g. gate 300/301 to the
    # hall approach ramp) are built by this lane's module but no longer owned
    # here; they must belong to another asset in the scene, never be dropped silently.
    moved = sorted(set(shapes) - set(by_node))
    if moved:
        scene_groups = {int(o['source_node'].split('-')[1]): o.get('asset_group')
                        for o in bpy.data.collections[config['collection_name']].all_objects
                        if o.type == 'MESH' and str(o.get('source_node', '')).startswith('building-')}
        orphans = [n for n in moved if not scene_groups.get(n)]
        if orphans:
            raise ValueError(f'Geometry for nodes absent from the scene: {orphans}')
        print('Skipping nodes regrouped to other assets:', {n: scene_groups[n] for n in moved})
        shapes = {n: v for n, v in shapes.items() if n in by_node}
        info = {**info, 'regrouped_elsewhere': {f'building-{n:03}': scene_groups[n] for n in moved}}
    changes = []
    for node, shape in sorted(shapes.items()):
        if len(by_node[node]) != 1:
            raise ValueError(f'Expected one working mesh for building-{node:03}')
        obj = by_node[node][0]
        matrix = [list(r) for r in obj.matrix_world]
        props = {k: obj[k] for k in ('source_node', 'asset_group')}
        verts, faces = shape.world()
        stats = replace_mesh(obj, verts, faces, strict=info.get('strict', {}).get(node, True))
        if [list(r) for r in obj.matrix_world] != matrix or any(obj[k] != v for k, v in props.items()):
            raise ValueError(f'Identity or transform drift on {obj.name}')
        changes.append({'source_node': f'building-{node:03}', 'object': obj.name, **stats})
    states = {}
    for node, state in STATE_VISIBILITY.get(asset, {}).items():
        obj = by_node[node][0]
        obj.hide_render = state['hide_render']
        obj['display_state'] = state['state']
        states[f'building-{node:03}'] = state
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace / 'model.blend'))
    report = {'version': 1, 'asset_id': asset, 'recipe': str(Path(__file__).resolve()),
              'recipe_sha256': sha(__file__), 'module': MODULES[asset],
              'module_sha256': sha(HERE / (MODULES[asset] + '.py')),
              'geometry_helpers_sha256': sha(HERE / 'west_complex_geom.py'),
              'changed_objects': changes, 'state_visibility': states,
              'unchanged_owned_nodes': sorted(f'building-{n:03}' for n in set(by_node) - set(shapes)),
              **{k: v for k, v in info.items() if k != 'strict'}}
    (workspace / 'inspection').mkdir(exist_ok=True)
    (workspace / 'inspection/geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    report['mask_revisions'] = apply_mask_revisions(asset)
    (workspace / 'inspection/geometry-recipe.json').write_text(json.dumps(report, indent=2) + '\n')
    if packet:
        sys.path.insert(0, str(TOOLING))
        from refinement_workspace import modified
        result = modified(str(workspace))
        print(json.dumps(result, indent=2))
        closeup(workspace)


def closeup(workspace):
    """Supplemental tight-framed eight-view packet of the saved, reprojected model."""
    import shutil
    sys.path.insert(0, str(TOOLING))
    from refinement_workspace import _render
    config = json.loads((workspace / 'workspace.json').read_text())
    output = workspace / 'inspection/closeup'
    if output.exists():
        shutil.rmtree(output)
    _render(config, output, baseline=None)


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument('--round', type=int, default=None)
    parser.add_argument('--asset', required=True, choices=sorted(MODULES))
    parser.add_argument('--no-packet', action='store_true')
    args = parser.parse_args(argv)
    if args.round is not None:
        global ASSETS_DIR
        ASSETS_DIR = ASSETS_DIR.parents[1] / f'round-{args.round}/assets'
    apply(args.asset, packet=not args.no_packet)


if __name__ == '__main__':
    main()
