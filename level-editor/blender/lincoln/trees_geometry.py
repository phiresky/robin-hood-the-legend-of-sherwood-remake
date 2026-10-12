"""Lincoln tree, bush and scenery assets added to the grouped scene (no obstacles).

Run from the repository root after trees_inventory.py:

    /usr/bin/blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/trees_geometry.py -- \
      --output level-editor/work/needs-review/lincoln-refinement/trees/scene [asset ids ...]

Without asset ids every proposal in the tree catalog proposal is built. The script opens
lincoln-grouped-v5.blend (terrain ground with the stream cut), adds each asset to
``lincoln Working`` without touching other objects and saves ``lincoln-grouped-trees.blend``
plus ``scene-report.json``. Workspaces, packets and review files come from
prepare_assets.py and trees_handoff.py.

Foliage (source node ``foliage-<slug>``), in the style of the reviewed-good Leicester trees
and built with their recipes (imported, not copied):
- crown (projection_component ``crown``): ``foliage_trees.source_packet`` partitions the
  painted domain into lobes (exact source RGB, domain as physical alpha) and
  ``foliage_trees.refine_crown`` builds paired curved cutout cards per lobe (source front,
  neutral back) plus two transverse neutral cards. Every card lies on the vertical plane
  through the foot and is offset only along the source ray, so the source view is exact.
  Lobes spread along the ray over a round plan footprint as deep as the crown is wide
  (clipped at architecture, terrain and ground); low linear plantings stay shallow.
- wood (projection_component ``wood``): ``props_trees.geometry`` lofts a trunk from the terrain
  contact and forks toward the upper lobes (a short stem for bushes).
Scenery (``scenery-<slug>``): explicit plank/post geometry fitted to traced source corners.

Ground contact: the base pixel is cast along the source ray onto terrain only (the ``ground``
mesh, nodes owning authored ground domains, and rock/cliff/plateau groups) and walks up the
column while it sits under architecture. BASE_OVERRIDES hold visually chosen feet.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

ROOT = Path.cwd()
R = ROOT / 'level-editor/work/needs-review/lincoln-refinement'
sys.path.insert(0, str(ROOT / 'level-editor/blender/lincoln'))
# Leicester foliage recipes are the reviewed-good precedent; reused, not copied.
sys.path.insert(0, str(ROOT / 'level-editor/blender/leicester'))
from render_slots import acquire  # noqa: E402

VERSION = 'lincoln-foliage-cards-v6'
SCENE_BLEND = R / 'grouped/lincoln-grouped-v5.blend'
CATALOG = R / 'scratch/trees/inventory/tree-catalog-proposal.json'
MASKS = R / 'mask-review/source-masks-v5.json'
SOURCE = R / 'source-states/covered.png'
TOOLING_POINTER = R / 'tooling-trees/current.json'
W, H = 2944, 2176
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))
GROUND_DOMAINS = set(range(433, 453)) | {454}
# Rock/cliff/plateau groups without an authored ground domain still carry plants.
NATURAL_GROUP_WORDS = ('rock', 'cliff', 'plateau', 'slope', 'ridge', 'hillside', 'spur', 'ledge', 'bank')
# Visually chosen trunk-foot pixels (x, y) where base_pixel()'s guess is wrong, mostly
# feet hidden behind walls. ground_contact() still walks up past architecture from here.
BASE_OVERRIDES = {
    # Foot hidden behind the north curtain; crown centred near x=2465.
    'lincoln-tree-north-plateau-west': (2465, 400),
    # Foot hidden behind the NE square tower.
    'lincoln-tree-north-plateau-east': (2690, 449),
}
# Visually chosen feet that must not walk (they already stand on the ledge they grow from).
FIXED_FEET = {
    # Painted on the rock spur behind the bailey thatched cottage; the column walk
    # otherwise climbs past the keep terrace to native z 422.
    'lincoln-bush-bailey-rock': (1440, 1330),
    # Both stand in the north bailey in front of the curtain; the painted foot is visible
    # grass, so the walk-up (which pushed them back into the curtain) is not wanted.
    'lincoln-tree-north-curtain-west': (2110, 603),
    'lincoln-tree-north-bailey-rim': (2331, 496),
}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def pixel_of(point):
    """Source pixel (x, y) of a Blender world point."""
    return point[0], -point[1] * SIN - point[2] * COS


def world_on_plane(x, y, ground):
    """Point with source pixel (x, y) on the vertical plane native y = ground."""
    from mathutils import Vector
    return Vector((x, -ground / SIN, (ground - y) / COS))


def terrain_nodes():
    manifest = json.loads(MASKS.read_text())
    entries = [e for p in manifest['projections'].values() for e in p['assignments']]
    return {e['source_node'] for e in entries if set(e['mask_indices']) & GROUND_DOMAINS} | {'ground'}


def load_domain(row):
    import numpy as np
    from PIL import Image
    bitmap = np.asarray(Image.open(row['domain_png']).convert('L')) > 0
    x, y = row['domain_box_top_left']
    full = np.zeros((H, W), bool)
    full[y:y + bitmap.shape[0], x:x + bitmap.shape[1]] = bitmap
    return full


def node_for(row):
    return ('scenery-' if row['kind'] == 'scenery' else 'foliage-') + row['id'].removeprefix('lincoln-')


def terrain_hit(terrain_tree, pixel):
    from mathutils import Vector
    toward = Vector((0, -COS, SIN))
    bx, by = pixel
    start = Vector((bx + .5, 0, -(by + .5) / COS)) + toward * 20000
    hit, _, _, _ = terrain_tree.ray_cast(start, -toward)
    if hit is None:
        raise ValueError(f'No terrain under base pixel {pixel}')
    return hit


def base_pixel(domain):
    """Foot pixel guess: the painted trunk if the lowest rows are narrow, else the
    column centroid of the crown's lower third, at the lowest painted row."""
    import numpy as np
    ys, xs = np.nonzero(domain)
    bottom, top = int(ys.max()), int(ys.min())
    width = xs.max() - xs.min() + 1
    low = xs[ys >= bottom - 6]
    if low.max() - low.min() + 1 <= .25 * width:
        return [int(np.median(low)), bottom], 'painted trunk foot (narrow lowest rows)'
    third = xs[ys >= bottom - (bottom - top) / 3]
    return [int(round(third.mean())), bottom], 'lower-third crown centroid at lowest painted row'


def ground_contact(terrain_tree, architecture_tree, base, depth_clearance):
    """Walk the foot up the source column until it and the crown's near half stand
    on open terrain, not inside/under architecture that the crown merely peeks over."""
    from mathutils import Vector
    up = Vector((0, 0, 1))
    bx, by = base
    for y in range(by, max(by - 400, 0), -2):
        foot = terrain_hit(terrain_tree, (bx, y))
        blocked = False
        for dy in (0., -.5 * depth_clearance, -depth_clearance):
            probe = foot + Vector((0, dy, 1))
            if architecture_tree.ray_cast(probe, up)[0] is not None:
                blocked = True
                break
        if not blocked:
            return foot, (bx, y), by - y
    raise ValueError(f'No open terrain above base pixel {base}')


def mesh_object(name, verts, faces, collection, props):
    import bpy
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([tuple(v) for v in verts], [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    for key, value in props.items():
        obj[key] = value
    return obj


def source_uv_and_material(obj, image):
    """Fallback source-projection UV and material (the ownership bake replaces it)."""
    import bpy
    mesh = obj.data
    layer = mesh.uv_layers.new(name='source projection')
    for loop in mesh.loops:
        px, py = pixel_of(obj.matrix_world @ mesh.vertices[loop.vertex_index].co)
        layer.data[loop.index].uv = (px / W, 1 - py / H)
    material = bpy.data.materials.new(obj.name + ' / fallback')
    material.use_nodes = True
    nodes = material.node_tree.nodes
    texture = nodes.new('ShaderNodeTexImage')
    texture.image = image
    material.node_tree.links.new(texture.outputs['Color'], nodes['Principled BSDF'].inputs['Base Color'])
    mesh.materials.append(material)


def topology(obj):
    import bmesh
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    report = dict(vertices=len(bm.verts), faces=len(bm.faces),
                  non_manifold_edges=sum(not e.is_manifold for e in bm.edges),
                  zero_area_faces=sum(f.calc_area() < 1e-8 for f in bm.faces))
    bm.free()
    return report


def kmeans_seeds(domain, count, rounds=25):
    """Deterministic lobe seeds: farthest-point initialisation plus Lloyd rounds."""
    import numpy as np
    ys, xs = np.nonzero(domain)
    points = np.stack([xs, ys], 1).astype(float)
    sample = points[::max(1, len(points) // 20000)]
    seeds = [sample[np.argmin(sample[:, 1])]]
    for _ in range(1, min(count, len(sample))):
        distance = np.min([((sample - s) ** 2).sum(1) for s in seeds], 0)
        seeds.append(sample[np.argmax(distance)])
    seeds = np.array(seeds)
    for _ in range(rounds):
        label = np.argmin(((sample[:, None] - seeds[None]) ** 2).sum(2), 1)
        seeds = np.array([sample[label == k].mean(0) if np.any(label == k) else seeds[k] for k in range(len(seeds))])
    return [(int(round(x)), int(round(y))) for x, y in seeds]


def lobe_plan(domain, kind):
    """Lobe count, card offset scale and depth rule, sized to the plant.

    Leicester tuned 8 lobes and +-20 px card offsets for ~250 px trees. Smaller plants get
    fewer lobes and proportionally smaller offsets; low linear plantings (beds, hedges,
    wide flat thickets) get flat lobes whose depth follows their height, not their width.
    """
    import numpy as np
    ys, xs = np.nonzero(domain)
    width, height = xs.max() - xs.min() + 1, ys.max() - ys.min() + 1
    count = int(min(8, max(2, round(math.sqrt(len(xs)) / 18))))
    scale = min(1., max(width, height) / 250.)
    # Trees keep round crowns even when wide (several are cut by the map edge).
    linear = kind != 'tree' and width >= 1.75 * height
    if linear:
        shape = 'linear-flat'
    else:
        shape = 'tree-round' if kind == 'tree' else 'bush-round'
    return dict(lobes=count, offset_scale=scale, shape=shape, width=int(width), height=int(height))


def foliage_packet(row, node_number, ground, plan, directory):
    """Leicester native-cutout partition (foliage_trees.source_packet) on a Lincoln domain."""
    import numpy as np
    import foliage_trees
    from PIL import Image
    directory.mkdir(parents=True, exist_ok=True)
    x, y = row['domain_box_top_left']
    width, height = Image.open(row['domain_png']).size
    inventory = dict(masks=[dict(index=node_number, png=str(Path(row['domain_png']).resolve()),
                                 box_top_left=[x, y], box_size=[width, height])])
    (directory / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
    (directory / 'masks.json').write_text(json.dumps(dict(mask_inventory='inventory.json',
                                                          projections=dict(exterior=dict(assignments=[]))), indent=2) + '\n')
    (directory / 'workspace.json').write_text(json.dumps(dict(source_mask_manifest=str(directory / 'masks.json'),
                                                              source_path=str(SOURCE)), indent=2) + '\n')
    domain = load_domain(row)
    foliage_trees.CONFIG[node_number] = dict(mask=node_number, ground=ground,
                                            canopy_bottom=int(np.nonzero(domain)[0].max()),
                                            seeds=kmeans_seeds(domain, plan['lobes']))
    evidence = foliage_trees.source_packet(directory, node_number, directory / 'lobes')
    for lobe in evidence['lobes']:
        x0, y0, x1, y1 = lobe['bbox_source']
        if plan['shape'] == 'linear-flat':
            lobe['depth_radius'] = .35 * min(x1 - x0, y1 - y0)
    return evidence


def wood_paths(domain, foot_on_plane, seeds, width, kind):
    """props_trees BRANCHES centrelines (x, plane y, radius): a vertical trunk from the
    foot forking toward the upper lobes; bushes get a short hidden stem and two forks."""
    import numpy as np
    from scipy import ndimage as nd
    ys, xs = np.nonzero(domain)
    fx, fy = foot_on_plane
    # Wood must not show where the painting shows ground: each sample's radius is
    # capped by the painted half-width at its source pixel (1 px inside holes).
    halfwidth = nd.distance_transform_edt(domain)
    cx, cy = float(xs.mean()), float(ys.mean())
    radius = min(max(.045 * width, 3.), 11.) if kind == 'tree' else min(max(.03 * width, 1.5), 4.)
    rise = .45 if kind == 'tree' else .25
    trunk = [(fx, fy + 4, radius * 1.2), (fx, fy + rise * (cy - fy), radius * .9),
             (fx + .25 * (cx - fx), cy + (.15 if kind == 'tree' else .5) * (fy - cy), radius * .65)]
    paths = [trunk]
    top = trunk[-1]
    for sx, sy in sorted(seeds, key=lambda s: s[1])[:3 if kind == 'tree' else 2]:
        paths.append([top[:2] + (radius * .5,), ((top[0] + sx) / 2, (top[1] + sy) / 2, radius * .4),
                      (sx, sy, radius * .15)])

    def capped(x, y, r):
        ix, iy = int(round(x)), int(round(y))
        painted = halfwidth[iy, ix] if 0 <= ix < W and 0 <= iy < H else 0.
        return (x, y, float(min(r, max(painted, 1.))))
    # The first trunk sample is below the foot (underground, hidden by terrain); keep it.
    return [[path[0] if path is trunk and i == 0 else capped(*sample) for i, sample in enumerate(path)]
            for path in paths]


def build_foliage(row, terrain_tree, architecture_tree, static_tree, collection, image, directory, node_number):
    import numpy as np
    node = node_for(row)
    domain = load_domain(row)
    base, base_rule = base_pixel(domain)
    if row['id'] in BASE_OVERRIDES:
        base, base_rule = list(BASE_OVERRIDES[row['id']]), 'visual override'
    ys, xs = np.nonzero(domain)
    width = float(xs.max() - xs.min())
    if row['id'] in FIXED_FEET:
        foot_pixel, walked = tuple(FIXED_FEET[row['id']]), 0
        foot, base_rule = terrain_hit(terrain_tree, foot_pixel), 'visual fixed foot (no walk)'
    else:
        try:
            foot, foot_pixel, walked = ground_contact(terrain_tree, architecture_tree, base, .3 * width)
        except ValueError:
            # Plants on ledges under overhanging architecture: require only the foot itself open.
            foot, foot_pixel, walked = ground_contact(terrain_tree, architecture_tree, base, 0.)
            base_rule += '; crown depth clearance waived (overhang)'
    # Leicester convention: cards lie on the vertical plane native y = ground.
    ground0 = -foot.y * SIN
    plan = lobe_plan(domain, row['kind'])
    props = dict(source_node=node, asset_group=row['id'], asset_name=row['name'],
                 part_name='Painted ' + row['kind'], foliage_recipe=VERSION, foliage_kind=row['kind'])
    samples = list(zip(xs[::5], ys[::5]))
    shift, attempts = 0., []
    while True:
        ground = ground0 + shift
        if shift:
            # Moved toward the camera along the source rays (projection unchanged): the
            # foot drops vertically from the plane at the base pixel onto whatever is below.
            foot = drop_to_surface(static_tree, world_on_plane(foot_pixel[0], foot_pixel[1], ground))
        crown, wood, crown_report, evidence, paths = build_parts(row, node_number, ground, foot, plan, props,
                                                                 collection, image, directory, domain, width, static_tree)
        hidden = hidden_fraction([crown, wood], static_tree, samples)
        attempts.append(dict(card_plane_native_y=ground, shift=shift, hidden_domain_fraction=hidden))
        if hidden <= .05 or shift >= MAX_SHIFT:
            break
        # Rebuild in front: remove this attempt's meshes (and their unused data).
        for obj in (crown, wood):
            mesh = obj.data
            bpy_remove(obj)
            if mesh.users == 0:
                import bpy
                bpy.data.meshes.remove(mesh)
        shift += SHIFT_STEP
    native_foot_z = foot.z * COS
    return dict(asset_id=row['id'], kind=row['kind'], source_node=node, base_pixel=list(base), base_pixel_rule=base_rule,
                foot_pixel=list(foot_pixel), foot_walked_up_pixels=walked, ground_contact_world=list(foot),
                ground_contact_native_z=native_foot_z, card_plane_native_y=ground, lobe_plan=plan,
                depth_fit=attempts,
                wood_paths=paths, lobes=len(evidence['lobes']),
                crown={k: v for k, v in crown_report.items() if k != 'source_partition'} | dict(name=crown.name),
                wood=dict(name=wood.name, **topology(wood)))


# Depth fit: when other geometry hides more than 5% of a plant's painted pixels, move its
# card plane toward the camera in SHIFT_STEP native-y steps (source projection unchanged).
SHIFT_STEP, MAX_SHIFT = 12., 360.


def bpy_remove(obj):
    import bpy
    bpy.data.objects.remove(obj, do_unlink=True)


def drop_to_surface(static_tree, point, lift=2.):
    from mathutils import Vector
    hit = static_tree.ray_cast(point + Vector((0, 0, lift)), Vector((0, 0, -1)))[0]
    if hit is None:
        raise ValueError(f'No surface below {tuple(point)}')
    return hit


def hidden_fraction(objects, static_tree, samples):
    """Fraction of sampled domain pixels where static geometry is hit before the plant
    (plant hits honour physical cutout alpha)."""
    from mathutils import Vector
    from mathutils.bvhtree import BVHTree
    from physical_opacity import OpacityRegistry
    import bpy
    bpy.context.view_layer.update()
    opacity = OpacityRegistry()
    verts, tris = [], []
    for obj in objects:
        obj.data.calc_loop_triangles()
        offset = len(verts)
        verts += [obj.matrix_world @ v.co for v in obj.data.vertices]
        for tri in obj.data.loop_triangles:
            opacity.add(obj, obj.data, tri)
            tris.append(tuple(offset + i for i in tri.vertices))
    own = opacity.wrap(BVHTree.FromPolygons(verts, tris, all_triangles=True))
    toward = Vector((0, -COS, SIN))
    hidden = seen = 0
    for x, y in samples:
        start = Vector((x + .5, 0, -(y + .5) / COS)) + toward * 20000
        mine = own.ray_cast(start, -toward)
        if mine[0] is None:
            continue
        seen += 1
        other = static_tree.ray_cast(start, -toward)
        if other[0] is not None and other[3] < mine[3]:
            hidden += 1
    return hidden / max(seen, 1)


def round_depths(evidence, ground, domain, static_tree):
    """Per-lobe (offset, radius) along the source ray so the crown is about as deep as it is
    wide: lobes spread over a round plan footprint of the crown's visible width, alternating
    front and back. Each lobe's ray extent is clipped where architecture or terrain lies in
    front of or behind it, and where its back would sink into the ground. Returns
    (depths, radii, clips); a lobe occupies [depth - radius, depth + radius] along the ray.
    """
    import numpy as np
    from mathutils import Vector
    toward = Vector((0, -COS, SIN))
    ys, xs = np.nonzero(domain)
    centre_x, half_width = float(xs.mean()), (xs.max() - xs.min() + 1) / 2.
    depths, radii, clips = [], [], []
    for number, lobe in enumerate(evidence['lobes']):
        x0, y0, x1, y1 = lobe['bbox_source']
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        # Plan-view half depth available at this lobe's x on a disc of the crown's width,
        # converted to distance along the source ray (horizontal = cos35 * ray distance).
        half_depth = math.sqrt(max(half_width ** 2 - (cx - centre_x) ** 2, (.35 * half_width) ** 2)) / COS
        own = .55 * max(x1 - x0, y1 - y0)
        sign = 1 if number % 2 == 0 else -1
        depth, radius = sign * .5 * half_depth, max(own, .5 * half_depth)
        low, high = depth - radius, depth + radius
        centre = world_on_plane(cx, cy, ground)
        clip = []
        ahead = static_tree.ray_cast(centre, toward)
        if ahead[0] is not None and ahead[3] - 2 < high:
            high = max(ahead[3] - 2, 2.); clip.append('front architecture/terrain')
        behind = static_tree.ray_cast(centre, -toward)
        if behind[0] is not None and -(behind[3] - 2) > low:
            low = min(-(behind[3] - 2), -2.); clip.append('back architecture/terrain')
        # The back card's lowest edge must stay above the ground below it.
        bottom = world_on_plane(cx, y1, ground)
        for _ in range(40):
            point = bottom + toward * low
            surface = static_tree.ray_cast(point + Vector((0, 0, 400)), Vector((0, 0, -1)))[0]
            if surface is None or surface.z <= point.z or low >= -2:
                break
            low = min(low + 4., -2.); clip.append('back ground') if 'back ground' not in clip else None
        depths.append((low + high) / 2)
        radii.append(max((high - low) / 2, 2.))
        clips.append(clip)
    return depths, radii, clips


def build_parts(row, node_number, ground, foot, plan, props, collection, image, directory, domain, width, static_tree):
    import foliage_trees
    import props_trees
    original_depths = list(foliage_trees.DEPTHS)
    try:
        if plan['shape'] == 'linear-flat':
            # Hedges, beds and fences: the painting shows a line; keep them shallow.
            foliage_trees.DEPTHS[:] = [d * plan['offset_scale'] for d in original_depths]
            evidence = foliage_packet(row, node_number, ground, plan, directory)
            clips = []
        else:
            evidence = foliage_packet(row, node_number, ground, plan, directory)
            depths, radii, clips = round_depths(evidence, ground, domain, static_tree)
            foliage_trees.DEPTHS[:] = depths + original_depths[len(depths):]
            for lobe, radius in zip(evidence['lobes'], radii):
                lobe['depth_radius'] = radius
        crown = mesh_object(row['name'] + ' / crown', [], [], collection, {**props, 'projection_component': 'crown'})
        crown_report = foliage_trees.refine_crown(crown, node_number, evidence)
        crown_report['lobe_depth_clips'] = clips
        crown_report['lobe_depths'] = list(foliage_trees.DEPTHS[:len(evidence['lobes'])])
    finally:
        foliage_trees.DEPTHS[:] = original_depths
    paths = wood_paths(domain, (foot.x, ground - foot.z * COS), foliage_trees.CONFIG[node_number]['seeds'],
                       width, row['kind'])
    props_trees.SUPPORTED.add(node_number)
    props_trees.GROUND[node_number] = ground
    props_trees.CROWNS[node_number] = [[paths[0][-1][1], paths[0][-1][0] - 1, paths[0][-1][0] + 1]] * 2
    props_trees.BRANCHES[node_number] = [list(path) for path in paths]
    verts, faces = props_trees.geometry(node_number, 'wood')
    wood = mesh_object(row['name'] + ' / wood', verts, faces, collection, {**props, 'projection_component': 'wood'})
    source_uv_and_material(wood, image)
    return crown, wood, crown_report, evidence, paths


# Landing stage corners traced on the source (x, y): deck back-left, back-right,
# front-right, front-left; front-left post foot; mooring-pole top and foot.
LANDING = dict(deck=[(486, 630), (529, 621), (543, 648), (506, 662)], deck_thickness=3.,
               post_foot=(508, 677), pole_top=(542, 614), pole_foot=(542, 672))


def box_between(a, b, radius, sides=8):
    """Closed vertical-ish prism (octagonal) from world point a to b."""
    from mathutils import Vector
    axis = (b - a).normalized()
    side = axis.cross(Vector((0, 1, 0))).normalized() if abs(axis.y) < .9 else axis.cross(Vector((1, 0, 0))).normalized()
    up = axis.cross(side).normalized()
    verts = [p + radius * (side * math.cos(k * math.tau / sides) + up * math.sin(k * math.tau / sides))
             for p in (a, b) for k in range(sides)]
    faces = [tuple(reversed(range(sides))), tuple(range(sides, 2 * sides))]
    faces += [(k, (k + 1) % sides, sides + (k + 1) % sides, sides + k) for k in range(sides)]
    return verts, faces


def build_landing_stage(row, terrain_tree, static_tree, collection, image):
    """Plank deck at the height that places its painted front-left post on the terrain.

    If the bank hides the deck from the source camera, the whole stage moves toward the
    camera along the source rays (projection unchanged); posts drop vertically to the ground.
    """
    from mathutils import Vector
    node = node_for(row)
    toward = Vector((0, -COS, SIN))
    post_foot = terrain_hit(terrain_tree, LANDING['post_foot'])
    foot_native_z = post_foot.z * COS
    # The painted post runs from the deck's front-left corner straight down to its foot,
    # so the deck stands (post pixel length) native units above that foot.
    deck_z = foot_native_z + (LANDING['post_foot'][1] - LANDING['deck'][3][1])
    base_corners = [Vector((x, -(y + deck_z) / SIN, deck_z / COS)) for x, y in LANDING['deck']]
    pole_top0 = terrain_hit(terrain_tree, LANDING['pole_foot']) + Vector((0, 0, (LANDING['pole_foot'][1] - LANDING['pole_top'][1]) / COS))
    down = Vector((0, 0, -LANDING['deck_thickness'] / COS))
    props = dict(source_node=node, asset_group=row['id'], asset_name=row['name'],
                 part_name='Landing stage and mooring pole', foliage_recipe=VERSION)
    domain = load_domain(row)
    import numpy as np
    ys, xs = np.nonzero(domain)
    samples = list(zip(xs[::2], ys[::2]))
    shift, attempts = 0., []
    while True:
        corners = [c + toward * shift for c in base_corners]
        verts = corners + [c + down for c in corners]
        faces = [(0, 1, 2, 3), (7, 6, 5, 4), (0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0)]
        parts = [mesh_object(row['name'] + ' / deck', verts, faces, collection, {**props, 'projection_component': 'deck'})]
        post_verts, post_faces = [], []
        for corner in corners:
            # Start above: a buried corner still finds the bank surface over it.
            ground = drop_to_surface(static_tree, corner + down, 200.)
            v, f = box_between(ground - Vector((0, 0, 4)), corner + down, 2.2)
            post_faces += [tuple(len(post_verts) + i for i in face) for face in f]
            post_verts += v
        pole_top = pole_top0 + toward * shift
        pole_foot = drop_to_surface(static_tree, pole_top, 0.)
        v, f = box_between(pole_foot - Vector((0, 0, 4)), pole_top, 2.)
        post_faces += [tuple(len(post_verts) + i for i in face) for face in f]
        post_verts += v
        parts.append(mesh_object(row['name'] + ' / posts', post_verts, post_faces, collection,
                                 {**props, 'projection_component': 'posts'}))
        hidden = hidden_fraction(parts, static_tree, samples)
        attempts.append(dict(shift_along_source_ray=shift, hidden_domain_fraction=hidden))
        if hidden <= .05 or shift >= 120:
            break
        for part in parts:
            mesh = part.data
            bpy_remove(part)
            import bpy
            bpy.data.meshes.remove(mesh)
        shift += 4.
    for part in parts:
        source_uv_and_material(part, image)
    return dict(asset_id=row['id'], kind='scenery', source_node=node, deck_native_z=corners[0].z * COS,
                post_foot_native_z=foot_native_z, pole_foot_native_z=pole_foot.z * COS, depth_fit=attempts,
                meshes=[dict(name=p.name, **topology(p)) for p in parts])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--keep-going', action='store_true', help='Record per-asset fitting failures and continue')
    parser.add_argument('assets', nargs='*')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    acquire()
    import bpy
    from mathutils.bvhtree import BVHTree
    sys.path.insert(0, json.loads(TOOLING_POINTER.read_text())['directory'])
    output = args.output.resolve()
    blend = output / 'lincoln-grouped-trees.blend'
    if blend.exists():
        raise FileExistsError(blend)
    output.mkdir(parents=True, exist_ok=True)
    catalog = json.loads(CATALOG.read_text())
    rows = {row['id']: row for row in catalog['assets']}
    missing = set(args.assets) - set(rows)
    if missing:
        raise ValueError(f'Unknown proposed assets: {sorted(missing)}')
    selected = [rows[a] for a in (args.assets or rows)]
    bpy.ops.wm.open_mainfile(filepath=str(SCENE_BLEND), load_ui=False)
    bpy.context.window.scene = bpy.data.scenes['lincoln Refinement']
    collection = bpy.data.collections['lincoln Working']
    if any(str(o.get('source_node', '')).startswith(('foliage-', 'scenery-')) for o in collection.all_objects):
        raise ValueError('Scene already contains foliage or scenery assets')
    nodes = terrain_nodes()
    terrain = [o for o in collection.all_objects if o.type == 'MESH' and not o.hide_render
               and (o.get('source_node') in nodes or any(word in str(o.get('asset_group', '')) for word in NATURAL_GROUP_WORDS))]

    def bvh(objects):
        verts, tris = [], []
        for obj in objects:
            obj.data.calc_loop_triangles()
            offset = len(verts)
            verts += [obj.matrix_world @ v.co for v in obj.data.vertices]
            tris += [tuple(offset + i for i in t.vertices) for t in obj.data.loop_triangles]
        return BVHTree.FromPolygons(verts, tris, all_triangles=True)
    terrain_tree = bvh(terrain)
    architecture_tree = bvh([o for o in collection.all_objects
                             if o.type == 'MESH' and not o.hide_render and o not in terrain])
    # Everything already in the scene (no foliage yet): depth-fit occluders and foot surfaces.
    static_tree = bvh([o for o in collection.all_objects if o.type == 'MESH' and not o.hide_render])
    image = bpy.data.images.load(str(SOURCE), check_existing=True)
    reports, failures = [], []
    for number, row in enumerate(selected):
        try:
            if row['kind'] == 'scenery':
                reports.append(build_landing_stage(row, terrain_tree, static_tree, collection, image))
            else:
                # Lobe partition ids only key the Leicester recipe tables; they are not mask indices.
                reports.append(build_foliage(row, terrain_tree, architecture_tree, static_tree, collection, image,
                                             output / 'foliage-source' / row['id'], 10000 + number))
        except ValueError as error:
            if not args.keep_going:
                raise
            failures.append(dict(asset_id=row['id'], error=str(error)))
    bpy.context.view_layer.update()
    bpy.ops.wm.save_as_mainfile(filepath=str(blend), compress=True)
    # Scene inventory for catalog validation (prepare_assets): every mesh incl. the new parts.
    from refinement_inventory import inventory
    scene_inventory = inventory(output / 'inventory', collection_name='lincoln Working', map_name='lincoln',
                                source_path=str(SOURCE))
    summary = dict(version=1, recipe=str(Path(__file__).resolve()), recipe_sha256=sha(__file__), recipe_version=VERSION,
                   scene=str(SCENE_BLEND), scene_sha256=sha(SCENE_BLEND), catalog=str(CATALOG),
                   catalog_sha256=sha(CATALOG), terrain_nodes=sorted(nodes),
                   terrain_objects=sorted(o.name for o in terrain), output_blend=str(blend),
                   output_blend_sha256=sha(blend), inventory=scene_inventory, assets=reports, failures=failures)
    (output / 'scene-report.json').write_text(json.dumps(summary, indent=2, default=list) + '\n')
    print('TREES_SCENE_COMPLETE', len(reports), 'built', len(failures), 'failed', flush=True)


if __name__ == '__main__':
    main()
