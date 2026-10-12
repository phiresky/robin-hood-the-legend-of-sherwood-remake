"""Coverage audit, review.md and candidate.json for prepared Lincoln tree workspaces.

Run from the repository root after prepare_assets.py has created trees/assets/<id>:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/trees_handoff.py -- [asset ids ...]

One Blender process opens the trees scene once and, per asset, casts a source-camera ray
through every proposed domain pixel (alpha-aware: rays pass through cutout leaf gaps) and
through the pixels around it. The domain comes from the inventory proposal, independently of
the acceptance masks. Shared foliage pixels are decided by first hit (the front plant owns
them); every contested pair is listed in review.md so the user can override it in the gallery.
Hashes bind the workspace model.blend and modified/views.json.
"""
import hashlib
import json
import math
import sys
from pathlib import Path

ROOT = Path.cwd()
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
T = R / 'trees'
sys.path.insert(0, str(ROOT / 'level-editor/blender/lincoln'))
from render_slots import acquire  # noqa: E402

SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
W, H = 2944, 2176
RECIPE = ROOT / 'level-editor/blender/lincoln/trees_geometry.py'
PASS_LIMIT = .08  # exclusive-foreign plus missed domain pixels, fraction of the domain


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_domain(row):
    import numpy as np
    from PIL import Image
    bitmap = np.asarray(Image.open(row['domain_png']).convert('L')) > 0
    x, y = row['domain_box_top_left']
    full = np.zeros((H, W), bool)
    full[y:y + bitmap.shape[0], x:x + bitmap.shape[1]] = bitmap
    return full


def scene_tree(collection):
    from mathutils.bvhtree import BVHTree
    from physical_opacity import OpacityRegistry
    opacity = OpacityRegistry()
    verts, tris, owners = [], [], []
    for obj in collection.all_objects:
        if obj.type != 'MESH' or obj.hide_render:
            continue
        obj.data.calc_loop_triangles()
        offset = len(verts)
        verts += [obj.matrix_world @ v.co for v in obj.data.vertices]
        for tri in obj.data.loop_triangles:
            opacity.add(obj, obj.data, tri)
            tris.append(tuple(offset + i for i in tri.vertices))
            owners.append(obj.get('asset_group') or obj.get('source_node'))
    return opacity.wrap(BVHTree.FromPolygons(verts, tris, all_triangles=True)), owners


def audit(row, tree, owners, domains, output):
    import numpy as np
    from mathutils import Vector
    from PIL import Image
    toward = Vector((0, -COS, SIN))
    domain = domains[row['id']]
    x0, y0, x1, y1 = row['source_box']
    pad = 40
    bx0, by0, bx1, by1 = max(0, x0 - pad), max(0, y0 - pad), min(W, x1 + pad), min(H, y1 + pad)
    counts = dict(own_first_hit=0, shared_front_foliage=0, foreign_first_hit=0, miss=0)
    foreign, contested_lost, contested_won = {}, {}, {}
    over = 0
    picture = np.zeros((by1 - by0, bx1 - bx0, 3), np.uint8)
    for y in range(by0, by1):
        for x in range(bx0, bx1):
            start = Vector((x + .5, 0, -(y + .5) / COS)) + toward * 20000
            hit, _, index, _ = tree.ray_cast(start, -toward)
            owner = owners[index] if hit is not None else None
            if not domain[y, x]:
                if owner == row['id']:
                    over += 1; picture[y - by0, x - bx0] = (255, 200, 0)
                    for other, other_domain in domains.items():
                        if other != row['id'] and other_domain[y, x]:
                            contested_won[other] = contested_won.get(other, 0) + 1
                continue
            if owner is None:
                counts['miss'] += 1; picture[y - by0, x - bx0] = (255, 0, 0)
            elif owner == row['id']:
                counts['own_first_hit'] += 1; picture[y - by0, x - bx0] = (0, 255, 0)
                for other, other_domain in domains.items():
                    if other != row['id'] and other_domain[y, x]:
                        contested_won[other] = contested_won.get(other, 0) + 1
            elif owner in domains and domains[owner][y, x]:
                counts['shared_front_foliage'] += 1; picture[y - by0, x - bx0] = (0, 120, 255)
                contested_lost[owner] = contested_lost.get(owner, 0) + 1
            else:
                counts['foreign_first_hit'] += 1; picture[y - by0, x - bx0] = (255, 0, 255)
                foreign[owner] = foreign.get(owner, 0) + 1
    source = np.asarray(Image.open(R / 'source-states/covered.png').convert('RGB'))[by0:by1, bx0:bx1].astype(float)
    marked = picture.any(-1)
    blend = source.copy()
    blend[marked] = source[marked] * .35 + picture[marked] * .65
    sheet = np.concatenate([source, blend], 1).astype(np.uint8)
    output.mkdir(parents=True, exist_ok=True)
    path = output / 'coverage-source-camera.png'
    Image.fromarray(sheet).resize((sheet.shape[1] * 2, sheet.shape[0] * 2), Image.NEAREST).save(path)
    total = int(domain.sum())
    return dict(domain_pixels=total, **counts, foreign_first_hit_by_owner=foreign,
                contested_lost_to=contested_lost, contested_won_from=contested_won,
                own_fraction=counts['own_first_hit'] / total, overreach_pixels_outside_domain=over,
                crop=[bx0, by0, bx1, by1], image=str(path), image_sha256=sha(path),
                legend=('left: source; right: green own first hit, blue shared foliage pixel won by the plant in '
                        'front, magenta other geometry first, red miss, orange this plant covers a pixel outside its domain'))


def write(row, scene_row, coverage, workspace):
    model, views = workspace / 'model.blend', workspace / 'modified/views.json'
    validation = json.loads((workspace / 'validation.json').read_text())
    if validation.get('status') != 'PASS':
        raise ValueError(f'Workspace validation failed: {workspace}')
    exclusive = coverage['foreign_first_hit'] + coverage['miss']
    status = 'PASS' if exclusive <= PASS_LIMIT * coverage['domain_pixels'] else 'FAIL'
    rel_image = str(Path(coverage['image']).relative_to(workspace))
    contested = sorted(set(coverage['contested_lost_to']) | set(coverage['contested_won_from']))
    observation = (f"{coverage['own_first_hit']}/{coverage['domain_pixels']} domain pixels hit this asset first; "
                   f"{coverage['shared_front_foliage']} shared foliage pixels are won by a plant in front; "
                   f"{coverage['foreign_first_hit']} hit other geometry first {coverage['foreign_first_hit_by_owner']}; "
                   f"{coverage['miss']} miss; {coverage['overreach_pixels_outside_domain']} pixels outside the domain are "
                   'covered by this asset.')
    kind = scene_row['kind']
    limitations = [
        'Domain is the reviewed proposal from trees_inventory.py (native masks minus see-through architecture, or a traced/classified box).',
        'Rear and transverse foliage cards and unseen wood are neutral until the approved back-card texture fill.' if kind != 'scenery'
        else 'Deck underside, far posts and the pole back are neutral until texture fill.',
        'Cards are open render surfaces (Leicester foliage contract), not closed volumes; no collision or sight obstacle.' if kind != 'scenery'
        else 'Visual-only scenery part: no collision or sight obstacle.']
    audit_record = dict(
        version=1, asset_id=row['id'], status=status, model_sha256=sha(model), modified_views_sha256=sha(views),
        inspected_views=list(range(8)),
        method=('Source-camera first-hit ray (35 deg orthographic) per pixel of the proposed domain and of a 40 px margin, '
                'against every visible mesh of the trees scene with physical cutout alpha. The domain comes from the '
                'inventory proposal, not from the acceptance masks.'),
        observation=observation, evidence={coverage['image']: coverage['image_sha256'], str(views): sha(views)},
        limitations=limitations, coverage=coverage)
    (workspace / 'source-coverage-audit.json').write_text(json.dumps(audit_record, indent=2) + '\n')
    if kind == 'scenery':
        geometry = (f"Plank deck at native z {scene_row['deck_native_z']:.1f} on four posts and a mooring pole, fitted "
                    f"to traced source corners; post foot on the pond bank at native z {scene_row['post_foot_native_z']:.1f}.")
    else:
        plan = scene_row['lobe_plan']
        geometry = (f"{scene_row['lobes']} Leicester cutout lobes ({plan['shape']}, card offset scale "
                    f"{plan['offset_scale']:.2f}) and {len(scene_row['wood_paths'])} lofted wood paths; foot pixel "
                    f"{scene_row['foot_pixel']} ({scene_row['base_pixel_rule']}) on terrain at native z "
                    f"{scene_row['ground_contact_native_z']:.1f}.")
    lines = [f"# {row['name']} (`{row['id']}`)", '',
             f"Status: {'ready for user review' if status == 'PASS' else 'fix needed'}. Look approved by the user on "
             '2026-09-26 (Leicester cutout-card style). This asset has no sight obstacle.', '',
             '## Geometry', '', geometry, '',
             f"Native masks: {row['native_masks'] or 'none (authored domain)'}. Region: {row['region']}. {row['note']}".strip(), '',
             '## Source coverage', '', observation, '', f"Evidence: `{rel_image}`.", '']
    if contested:
        lines += ['## Contested foliage pixels (front plant owns them by default)', '',
                  'Override any of these in the gallery if the other plant should own the pixels.', '',
                  '| other asset | won by this asset | won by the other |', '|---|---|---|']
        for other in contested:
            lines.append(f"| `{other}` | {coverage['contested_won_from'].get(other, 0)} | {coverage['contested_lost_to'].get(other, 0)} |")
        lines.append('')
    lines += ['## Limitations', ''] + [f'- {item}' for item in limitations] + ['']
    (workspace / 'review.md').write_text('\n'.join(lines))
    candidate = dict(
        version=1, asset_id=row['id'], status='ready-for-user' if status == 'PASS' else 'fix-needed',
        geometry_refined=False, geometry_reviewed=True, inspected_views=list(range(8)),
        recipe=str(RECIPE), model_sha256=sha(model), modified_views_sha256=sha(views),
        changes=[geometry],
        no_change_reason=('New authored scenery asset: trees_geometry.py built it in the grouped scene before workspace '
                          'preparation, so the frozen baseline already is the reviewed geometry.'),
        limitations=limitations + ([f'Contested foliage pixels with {len(contested)} neighbouring plants; see review.md.']
                                   if contested else []),
        geometry_approval='pending', texture_generation='not-started', source_comparison=rel_image,
        user_feedback='Tree look approved by the user 2026-09-26 (Leicester cutout-card style).')
    (workspace / 'candidate.json').write_text(json.dumps(candidate, indent=2) + '\n')
    return status


def main():
    args = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    acquire()
    import bpy
    sys.path.insert(0, json.loads((R / 'tooling-trees/current.json').read_text())['directory'])
    proposal = {r['id']: r for r in json.loads((R / 'scratch/trees/inventory/tree-catalog-proposal.json').read_text())['assets']}
    import os
    scene = json.loads((T / os.environ.get('LINCOLN_TREES_SCENE', 'scene') / 'scene-report.json').read_text())
    scene_rows = {r['asset_id']: r for r in scene['assets']}
    ids = args or [i for i in proposal if (T / 'assets' / i / 'modified/views.json').exists()]
    bpy.ops.wm.open_mainfile(filepath=scene['output_blend'], load_ui=False)
    if sha(scene['output_blend']) != scene['output_blend_sha256']:
        raise ValueError('Trees scene changed since its report')
    tree, owners = scene_tree(bpy.data.collections['lincoln Working'])
    domains = {i: load_domain(r) for i, r in proposal.items()}
    results = {}
    for identifier in ids:
        workspace = T / 'assets' / identifier
        coverage = audit(proposal[identifier], tree, owners, domains, workspace / 'inspection')
        results[identifier] = write(proposal[identifier], scene_rows[identifier], coverage, workspace)
        print('TREES_HANDOFF', identifier, results[identifier], flush=True)
    print('TREES_HANDOFF_COMPLETE', json.dumps(dict(passed=sum(v == 'PASS' for v in results.values()),
                                                    failed=[k for k, v in results.items() if v != 'PASS'])), flush=True)


if __name__ == '__main__':
    main()
