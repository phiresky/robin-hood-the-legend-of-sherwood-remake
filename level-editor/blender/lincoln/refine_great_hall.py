"""Lincoln great-hall lane geometry recipe (hall, slate spire, hall-keep north wall,
south terrace, approach ramp).

Native obstacles are prisms extruded from native z = 0, although the hall stands on
the castle rock at native z ~ 420 and its neighbours at 230-420. This recipe restores
every owned mesh from the frozen workspace ``baseline.blend`` (so it is idempotent and
re-runnable with different ground heights), then cuts each owned pillar at the ground
height measured for it from the covered artwork and closes the cut with a cap face.

Run from the repository root:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
        --python level-editor/blender/lincoln/refine_great_hall.py -- --asset <asset-id> [--packet]
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
ROOT = HERE.parents[1] / 'work/needs-review/lincoln-refinement'
TOOLING = ROOT / 'tooling/e6b57cb851c7142b'
COS = math.cos(math.radians(35))
TAG = 'lincoln_great_hall_recipe'

# Native ground heights (native z; Blender Z = z / cos35). Evidence per value is in
# each asset's review.md and in inspection/ground-evidence.png.
HALL_GROUND = 420.0        # east wing / middle tower foot: masonry meets rock at pixel y ~1120
                           # on the south face (native y ~1550) -> z ~430; rock spur 465 top 422.
WEST_WING_GROUND = 360.0   # west-wing SW corner (1025,1327) masonry visible to pixel y ~965
                           # before garden wall / rocks hide it -> z <= 362.
HALL_FLOOR = 550.0         # interior floor slab 233 (flat top 550 across both wings).
TERRACE_GROUND = 400.0     # terrace south wall / corbel turret foot meets the rock at ~z 400.
RAMP_GROUND = 300.0        # ramp retaining wall: masonry visible below the parapet to pixel y ~1325 at x 1300
                           # (front face y ~1642 -> z ~317) and ~1290 at x 1450 (y ~1587 -> z ~297).
NORTH_WALL_GROUND = 420.0  # hall-keep wall walk foot hidden behind the hall; rock spur 465 top 422.

DETAIL_MODULES = {
    'lincoln-great-hall': 'great_hall_hall',
    'lincoln-hall-south-terrace': 'great_hall_terrace',
    'lincoln-hall-approach-ramp': 'great_hall_ramp',
    'lincoln-hall-keep-north-wall': 'great_hall_north_wall',
}

WEST_WING = {'building-234', 'building-256', 'building-257', 'building-261', 'building-233'}
INTERIOR = {f'building-{n}' for n in (306, 307, 308, 309, 310, 311, 312, 313, 314, 315,
                                      317, 318, 320, 321, 322, 323, 324, 325)}


def ground_for(asset, node):
    if asset == 'lincoln-great-hall':
        if node in INTERIOR:
            return HALL_FLOOR, 'interior fixture on hall floor 233 (z 550)'
        if node in WEST_WING:
            return WEST_WING_GROUND, 'west-wing foot visible down the west hillside'
        return HALL_GROUND, 'hall foot on castle rock'
    if asset == 'lincoln-great-hall-slate-spire':
        return WEST_WING_GROUND, 'spire turret shaft runs down the west-wing SW face'
    if asset == 'lincoln-hall-keep-north-wall':
        return NORTH_WALL_GROUND, 'wall-walk foot on rock spur behind the hall'
    if asset == 'lincoln-hall-south-terrace':
        return TERRACE_GROUND, 'terrace walls on rock above the ramp'
    if asset == 'lincoln-hall-approach-ramp':
        return RAMP_GROUND, 'ramp retaining wall foot where masonry meets the cliff rock'
    raise ValueError(f'Not a great_hall lane asset: {asset}')


# Reviewed working-mask revisions for own nodes (workspace source-masks.json only; the
# shared manifest is untouched). Applied idempotently by the recipe.
WEST_HALL_FOLIAGE = [328, 333]   # garden hedge/bush silhouettes in front of the west-wing foot
MASK_REVISIONS = {
    'lincoln-great-hall': {
        'building-272': {
            'mask_indices': [253, 254, 373, 378, 413],
            'requires_first_hit_gating': True,
            'composite_envelope': ('Worker revision (great_hall lane): 272 is the east-wing NE roof-half sight slab '
                                   '(clipped under the east-wing roof planes). It receives east-hall masks 254/373/413 '
                                   'plus the original keep masks 253/378. First-hit gating decides between them; '
                                   'tree mask 158 stays excluded.'),
            'review_note': ('Working-mask revision for own node building-272: catalogued in lincoln-great-hall and lying '
                            'inside the east wing, under its gable roof. Keep-only masks left its east-wing pixels unowned. '
                            'Evidence: inspection/mask-272-evidence.png.'),
            'review_evidence': 'inspection/mask-272-evidence.png',
        },
    },
}


def _foliage_exclusion(entry):
    ex = sorted(set(entry.get('exclude_mask_indices') or []) | set(WEST_HALL_FOLIAGE))
    return {'exclude_mask_indices': ex, 'exclusions_reviewed': True,
            'exclusion_reason': ('Visually reviewed foreground silhouettes overlap this envelope. Garden hedge/bush masks '
                                 '328/333 (also excluded by the garden back wall) stand in front of the west-wing foot '
                                 'and are foreground foliage, not masonry (inspection/foreground-foliage-328-333.png). '
                                 'Earlier exclusions are kept.')}


def apply_mask_revisions(asset, workspace, part_ids):
    path = workspace / 'source-masks.json'
    data = json.loads(path.read_text())
    changed = []
    for entry in data['projections']['exterior']['assignments']:
        node = entry.get('source_node')
        if node not in part_ids:
            continue
        update = dict(MASK_REVISIONS.get(asset, {}).get(node, {}))
        if 'review_evidence' in update:
            update['review_evidence'] = str((workspace / update['review_evidence']).resolve())
        if asset == 'lincoln-great-hall' and set(entry.get('mask_indices', [])) & {251, 374}:
            update.update(_foliage_exclusion(entry))
        if update and any(entry.get(k) != v for k, v in update.items()):
            entry.update(update)
            changed.append(node)
    if changed:
        path.write_text(json.dumps(data, indent=2) + '\n')
    return changed


def _geometry_hash(obj):
    payload = {'m': [list(r) for r in obj.matrix_world],
               'v': [list(map(lambda c: round(c, 5), v.co)) for v in obj.data.vertices],
               'f': [list(p.vertices) for p in obj.data.polygons]}
    return hashlib.sha256(json.dumps(payload).encode()).hexdigest()


def restore_baseline(workspace, owned):
    """Replace owned mesh data with the frozen baseline mesh (idempotent re-runs)."""
    names = [o.name for o in owned]
    with bpy.data.libraries.load(str(workspace / 'baseline.blend'), link=False) as (src, dst):
        missing = set(names) - set(src.objects)
        if missing:
            raise ValueError(f'Owned objects missing from baseline: {sorted(missing)}')
        dst.objects = list(names)
    for obj, loaded in zip(owned, dst.objects):
        if loaded is None:
            raise ValueError(f'Failed to load baseline object for {obj.name}')
        if loaded.get('source_node') != obj.get('source_node'):
            raise ValueError(f'Baseline identity mismatch for {obj.name}')
        pairs = [(loaded.matrix_basis, obj.matrix_basis), (loaded.matrix_parent_inverse, obj.matrix_parent_inverse)]
        if any(abs(a - b) > 1e-6 for ma, mb in pairs for ra, rb in zip(ma, mb) for a, b in zip(ra, rb)):
            raise ValueError(f'Baseline transform differs for {obj.name}')
        old = obj.data
        new = loaded.data.copy()
        new.name = old.name
        obj.data = new
        bpy.data.objects.remove(loaded)
        if old.users == 0:
            bpy.data.meshes.remove(old)


def _cap_from_chains(bm, edges):
    """Cap an outline made of open chains (native side quads can be missing or gapped):
    walk the chains, join them end-to-nearest-start into one loop, and add one face."""
    adj = {}
    for e in edges:
        a, b = e.verts
        adj.setdefault(a, []).append(b)
        adj.setdefault(b, []).append(a)
    seen, chains = set(), []
    starts = [v for v, n in adj.items() if len(n) == 1] + list(adj)
    for s0 in starts:
        if s0 in seen:
            continue
        chain, cur, prev = [s0], s0, None
        seen.add(s0)
        while True:
            nxt = [n for n in adj[cur] if n is not prev and n not in seen]
            if not nxt:
                break
            prev, cur = cur, nxt[0]
            seen.add(cur)
            chain.append(cur)
        chains.append(chain)
    loop = chains.pop(0)
    while chains:
        end = loop[-1].co
        best = min(range(len(chains)), key=lambda i: min((chains[i][0].co - end).length, (chains[i][-1].co - end).length))
        c = chains.pop(best)
        if (c[-1].co - end).length < (c[0].co - end).length:
            c.reverse()
        loop.extend(c)
    if len(loop) < 3:
        return []
    try:
        face = bm.faces.new(loop)
    except ValueError:
        return []
    return [face]


def cut_below(obj, native_z):
    """Remove the part of obj below native_z (world) and cap the cut. Returns stats."""
    zw = native_z / COS
    mw = obj.matrix_world
    inv = mw.inverted()
    before_min = min((mw @ v.co).z for v in obj.data.vertices)
    if before_min >= zw - 1e-4:
        return {'cut': False, 'reason': 'already above ground'}
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    # Native prisms are stored as loose side quads with ~0.1-unit corner gaps and a
    # slightly lifted top; weld (0.3 Blender units) before cutting so
    # the cap can be filled and the shell stays closed.
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=0.3)
    open_before = sum(1 for e in bm.edges if not e.is_manifold)
    co = inv @ Vector((0, 0, zw))
    # A world plane n.(p - c) = 0 has local normal M^T n for p = M p_local + t.
    no =(mw.to_3x3().transposed() @ Vector((0, 0, 1))).normalized()
    geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
    res = bmesh.ops.bisect_plane(bm, geom=geom, plane_co=co, plane_no=no, clear_inner=True, dist=1e-5)
    # Weld the cut outline more aggressively (side quads of sloped native parts leave
    # gaps up to ~1 unit), then cap every closed boundary loop lying in the cut plane.
    on_plane = [v for v in bm.verts if abs((mw @ v.co).z - zw) < 1e-3]
    bmesh.ops.remove_doubles(bm, verts=on_plane, dist=1.5)
    on_plane = {v for v in bm.verts if abs((mw @ v.co).z - zw) < 1e-3}
    boundary = [e for e in bm.edges if e.is_boundary and all(v in on_plane for v in e.verts)]
    filled = bmesh.ops.holes_fill(bm, edges=boundary, sides=0)['faces'] if boundary else []
    if boundary and not filled:
        filled = _cap_from_chains(bm, boundary)
    bmesh.ops.dissolve_degenerate(bm, dist=1e-4, edges=list(bm.edges))
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.normal_update()
    open_edges = sum(1 for e in bm.edges if not e.is_manifold)
    degenerate = sum(1 for f in bm.faces if f.calc_area() < 1e-8)
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()
    obj[TAG] = 1
    obj['ground_native_z'] = native_z
    return {'cut': True, 'cap_faces': len(filled), 'open_edges_before': open_before, 'open_edges': open_edges, 'degenerate_faces': degenerate}


def refine(asset):
    owned = [o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == asset]
    if not owned:
        raise ValueError(f'No owned meshes for {asset}')
    return owned


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--asset', required=True)
    parser.add_argument('--packet', action='store_true', help='also regenerate modified/ with the frozen tooling')
    parser.add_argument('--round', type=int, default=1, choices=(1, 2, 3),
                        help='2: start from the round-2 baseline (which already holds the round-1 result) and '
                             'apply only the integrated-context fixes in great_hall_round2.py')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    workspace = ROOT / f'round-{args.round}/assets' / args.asset
    part_ids = set(json.loads((workspace / 'validation.json').read_text())['part_ids'])
    mask_changes = apply_mask_revisions(args.asset, workspace, part_ids)
    print(json.dumps({'mask_revisions': mask_changes}))
    from render_slots import acquire
    acquire()
    bpy.ops.wm.open_mainfile(filepath=str(workspace / 'model.blend'))
    outside = {o.name: _geometry_hash(o) for o in bpy.data.objects
               if o.type == 'MESH' and o.get('asset_group') != args.asset}
    owned = refine(args.asset)
    restore_baseline(workspace, owned)
    report = []
    if args.round >= 2:
        import great_hall_round2
        by_node = {o.get('source_node'): o for o in owned}
        fixes = great_hall_round2.apply(args.asset, by_node)
        for obj in sorted(owned, key=lambda o: o.get('source_node')):
            world = [obj.matrix_world @ v.co for v in obj.data.vertices]
            report.append({'object': obj.name, 'source_node': obj.get('source_node'), 'cut': False,
                           'round2_fix': fixes.get(obj.get('source_node')),
                           'vertices': len(world), 'faces': len(obj.data.polygons),
                           'native_z': [round(min(p.z for p in world) * COS, 2), round(max(p.z for p in world) * COS, 2)]})
    for obj in sorted(owned if args.round == 1 else [], key=lambda o: o.get('source_node')):
        node = obj.get('source_node')
        if not node or not obj.get('asset_group'):
            raise ValueError(f'{obj.name} lost its ownership properties')
        ground, why = ground_for(args.asset, node)
        stats = cut_below(obj, ground)
        world = [obj.matrix_world @ v.co for v in obj.data.vertices]
        report.append({'object': obj.name, 'source_node': node, 'ground_native_z': ground, 'reason': why,
                       'vertices': len(world), 'faces': len(obj.data.polygons),
                       'native_z': [round(min(p.z for p in world) * COS, 2), round(max(p.z for p in world) * COS, 2)],
                       **stats})
    # Per-asset detail passes (traced merlons, roofs, steps) live in great_hall_<part>.py.
    module_name = DETAIL_MODULES.get(args.asset) if args.round == 1 else None
    if module_name:
        import importlib
        module = importlib.import_module(module_name)
        by_node = {o.get('source_node'): o for o in owned}
        detail = module.apply(by_node)
        for row in report:
            if row['source_node'] in detail:
                obj = by_node[row['source_node']]
                world = [obj.matrix_world @ v.co for v in obj.data.vertices]
                row['detail'] = detail[row['source_node']]
                row['vertices'], row['faces'] = len(world), len(obj.data.polygons)
                row['native_z'] = [round(min(p.z for p in world) * COS, 2), round(max(p.z for p in world) * COS, 2)]
    for obj in owned:
        if not obj.get('source_node') or obj.get('asset_group') != args.asset:
            raise ValueError(f'{obj.name} lost its ownership properties')
    bpy.context.view_layer.update()
    after = {o.name: _geometry_hash(o) for o in bpy.data.objects
             if o.type == 'MESH' and o.get('asset_group') != args.asset}
    if outside != after:
        raise ValueError('Recipe changed an outside object')
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(workspace / 'model.blend'))
    result = {'version': 1, 'asset_id': args.asset, 'recipe': str(Path(__file__).resolve()),
              'objects': report, 'outside_objects_preserved': len(outside)}
    (workspace / 'geometry-recipe.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'asset': args.asset, 'cut': sum(r['cut'] for r in report),
                      'open_edges': sum(r.get('open_edges', 0) for r in report)}))
    if args.packet:
        sys.path.insert(0, str(TOOLING))
        import refinement_workspace
        packet = refinement_workspace.modified(str(workspace))
        print(json.dumps({'packet_status': packet.get('status') if isinstance(packet, dict) else str(packet)[:200]}))


if __name__ == '__main__':
    main()
