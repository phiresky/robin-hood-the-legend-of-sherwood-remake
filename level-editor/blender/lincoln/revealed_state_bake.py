"""Bake reviewed Lincoln revealed states onto state-only copies in a staged worker.

    blender --background --threads 2 --python-exit-code 1 \
      --python level-editor/blender/lincoln/revealed_state_bake.py -- \
      --stage-in <publication-N/stage-vM> --output <new directory> [--dry-run]

Inputs are the approved state specs (state-review/spec/<asset>.json), the per-state
sources and change regions (state-review/sources/) and the reviewed state masks
(state-review/masks/<asset>/source-masks.json). The stage-in worker is opened
read-only; the result is saved to <output>/worker.blend.

1. Each spec's state geometry is rebuilt in the staged worker (revealed_state_geometry
   build(), context copies skipped) and must match the approved state-model digests.
   State objects get their own copied material and image; faces created by cuts get new
   atlas islands in rows appended to that image (existing texels keep their exact rows).
2. For every primary (single-patch) state, a source-camera z-buffer of that state's
   visible map is built. A texel receives the state source RGB when its surface point is
   first-hit visible, faces the source, lies in the state's change region and inside
   the asset's reviewed state mask. Outside the change region the covered texels stay.
   Combination states (door open, winch shown) bake only the objects they alone show.
3. An approved object that receives revealed texels is never edited: a copy with its own
   material and image (an "appearance copy") takes the texels and is shown for the state,
   while the original gains that patch in reveal_hide_when_applied. When two states
   write the same object, the copy of the later patch in native order is shown when both
   are applied (the game composites that patch on top); each overlap is reported.
4. Verification: every object and image present in stage-in is byte-identical after the
   bake (geometry, UVs, material slots, image pixels); a source-camera render of each
   state's visible set is compared with the state source inside the change region.

Faces that no state source sees (cut caps, backs of cut walls) keep the neutral shade
(alpha 255, the bake's unknown convention) and are counted for optional generated fill.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np
from mathutils.bvhtree import BVHTree

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
WORK = ROOT / 'work/needs-review/lincoln-refinement'
REVIEW = WORK / 'state-review'
sys.path.insert(0, str(HERE))
import render_slots  # noqa: E402
import global_reproject as G  # noqa: E402
import revealed_state_geometry as RG  # noqa: E402

COLLECTION = G.COLLECTION
OWNED_UV = 'Owned source / exterior'
TAG = 'lincoln-revealed-state-bake-v1'
MIN_FACING = 0.05


def sha_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha(path):
    return sha_bytes(Path(path).read_bytes())


def image_digest(image):
    return sha_bytes(G.read_image(image).tobytes())


def object_fingerprint(obj, digests=None):
    """Geometry, UVs, material slot names and the pixels of every slot image.

    Pass a dict as digests to reuse image digests across objects: shared atlases
    (the 8192 combine atlas has hundreds of users) are then read once.
    """
    images = []
    for material in obj.data.materials:
        if material and material.node_tree:
            for n in material.node_tree.nodes:
                if n.type == 'TEX_IMAGE' and n.image:
                    if digests is None:
                        digest = image_digest(n.image)
                    else:
                        digest = digests.get(n.image.name) or digests.setdefault(n.image.name, image_digest(n.image))
                    images.append((n.image.name, digest))
    return RG.fingerprint(obj) + ':' + sha_bytes(json.dumps(images, sort_keys=True).encode())


def working_meshes():
    return [o for o in bpy.data.collections[COLLECTION].all_objects if o.type == 'MESH']


def shown(obj, applied):
    hide = set(obj.get('reveal_hide_when_applied', []))
    show = obj.get('reveal_show_when_applied')
    if hide & applied:
        return False
    if show is not None:
        return bool(set(show) & applied)
    return True


def record(obj):
    """Flat world-space triangle arrays for one mesh (global_reproject layout)."""
    mesh = obj.data
    mesh.calc_loop_triangles()
    co = np.empty(len(mesh.vertices) * 3)
    mesh.vertices.foreach_get('co', co)
    matrix = np.array(obj.matrix_world, dtype=np.float64)
    world = co.reshape(-1, 3) @ matrix[:3, :3].T + matrix[:3, 3]
    count = len(mesh.loop_triangles)
    vertices = np.empty(count * 3, dtype=np.int32)
    loops = np.empty(count * 3, dtype=np.int32)
    polygons = np.empty(count, dtype=np.int32)
    slots = np.empty(count, dtype=np.int32)
    mesh.loop_triangles.foreach_get('vertices', vertices)
    mesh.loop_triangles.foreach_get('loops', loops)
    mesh.loop_triangles.foreach_get('polygon_index', polygons)
    mesh.loop_triangles.foreach_get('material_index', slots)
    corners = world[vertices.reshape(-1, 3)]
    normals = np.cross(corners[:, 1] - corners[:, 0], corners[:, 2] - corners[:, 0])
    normals /= np.maximum(np.linalg.norm(normals, axis=1), 1e-12)[:, None]
    face_normals = np.empty(len(mesh.polygons) * 3)
    mesh.polygons.foreach_get('normal', face_normals)
    face_normals = face_normals.reshape(-1, 3) @ np.linalg.inv(matrix[:3, :3])
    face_normals /= np.maximum(np.linalg.norm(face_normals, axis=1), 1e-12)[:, None]
    return {'object': obj, 'corners': corners, 'loops': loops.reshape(-1, 3), 'polygons': polygons,
            'slots': slots, 'normals': normals, 'face_normals': face_normals}


class ZBuffer:
    """Source-camera first-hit buffer of the visible map, restricted to a pixel window."""

    def __init__(self, objects, window, size):
        x0, y0, x1, y1 = window
        width, height = size
        chunks = []
        for obj in objects:
            corners = record(obj)['corners']
            x, v, _ = G.screen(corners)
            keep = ((x.max(axis=1) >= x0) & (x.min(axis=1) <= x1) &
                    (v.max(axis=1) >= y0) & (v.min(axis=1) <= y1))
            if keep.any():
                chunks.append(corners[keep])
        self.corners = np.concatenate(chunks) if chunks else np.zeros((0, 3, 3))
        self.width, self.height = width, height
        self.ids, self.planes = G.rasterize(self, width, height)

    def visible(self, points):
        return G.visible(points, self.ids, self.planes, self.width, self.height)


def owned_binding(obj):
    """(slot, material, texture node, image, uv name) of the single owned-exterior slot in use."""
    used = sorted({p.material_index for p in obj.data.polygons})
    rows = []
    for slot in used:
        material = obj.data.materials[slot] if slot < len(obj.data.materials) else None
        textures = [n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image] \
            if material and material.node_tree else []
        uvs = [n for n in material.node_tree.nodes if n.type == 'UVMAP'] if material and material.node_tree else []
        if len(textures) != 1 or len(uvs) != 1:
            raise ValueError(f'{obj.name}: slot {slot} is not a single-image owned material')
        rows.append((slot, material, textures[0], textures[0].image, uvs[0].uv_map))
    if len(rows) != 1:
        raise ValueError(f'{obj.name}: expected exactly one used material slot, found {len(rows)}')
    return rows[0]


def isolate_material(obj, suffix):
    """Give obj its own copy of its single used material and image (pixels unchanged)."""
    slot, material, texture, image, uv = owned_binding(obj)
    material = material.copy()
    material.name = obj.name + ' / ' + suffix
    new_image = image.copy()
    new_image.name = obj.name + ' / ' + suffix
    next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image).image = new_image
    obj.data.materials[slot] = material
    material['revealed_state_bake'] = TAG
    return slot, material, new_image, uv


def pack_faces(obj, faces, uv_name, width, top):
    """Planar 1-texel-per-unit islands (2-texel gutter) for faces, shelf-packed above row top.

    Returns the rows used; UVs are written in texel units and normalized by the caller.
    """
    mesh = obj.data
    layer = mesh.uv_layers[uv_name]
    matrix = obj.matrix_world
    placements = []
    for index in faces:
        polygon = mesh.polygons[index]
        points = [matrix @ mesh.vertices[v].co for v in polygon.vertices]
        normal = (matrix.to_3x3() @ polygon.normal).normalized()
        axis = (points[1] - points[0])
        if axis.length < 1e-9:
            axis = normal.orthogonal()
        axis.normalize()
        other = normal.cross(axis).normalized()
        coords = [((p - points[0]).dot(axis), (p - points[0]).dot(other)) for p in points]
        umin = min(c[0] for c in coords)
        vmin = min(c[1] for c in coords)
        w = math.ceil(max(c[0] for c in coords) - umin) + 4
        h = math.ceil(max(c[1] for c in coords) - vmin) + 4
        placements.append((index, polygon, coords, umin, vmin, w, h))
    placements.sort(key=lambda row: -row[6])
    x = y = shelf = 0
    for index, polygon, coords, umin, vmin, w, h in placements:
        if w > width:
            raise ValueError(f'{obj.name}: face {index} wider than its atlas')
        if x + w > width:
            x, y, shelf = 0, y + shelf, 0
        for loop, (u, v) in zip(polygon.loop_indices, coords):
            layer.data[loop].uv = (x + 2 + u - umin, top + y + 2 + v - vmin)
        x += w
        shelf = max(shelf, h)
    return y + shelf


def uv_area_texels(obj, uv_name, size):
    layer = obj.data.uv_layers[uv_name]
    width, height = size
    areas = []
    for polygon in obj.data.polygons:
        pts = [(layer.data[l].uv[0] * width, layer.data[l].uv[1] * height) for l in polygon.loop_indices]
        area = 0.0
        for i in range(len(pts)):
            a, b = pts[i], pts[(i + 1) % len(pts)]
            area += a[0] * b[1] - a[1] * b[0]
        areas.append(abs(area) / 2)
    return areas


def prepare_state_object(obj, template):
    """Own material/image for a state object; cut or new faces get appended atlas islands."""
    mesh = obj.data
    if OWNED_UV not in mesh.uv_layers:
        # Additions: a fresh atlas using the template's owned material graph.
        slot, material, texture, image, uv = owned_binding(template)
        mesh.uv_layers.new(name=OWNED_UV)
        mesh.materials.clear()
        material = material.copy()
        material.name = obj.name + ' / revealed state'
        mesh.materials.append(material)
        for polygon in mesh.polygons:
            polygon.material_index = 0
        width = 256
        rows = pack_faces(obj, range(len(mesh.polygons)), OWNED_UV, width, 0)
        new_image = bpy.data.images.new(obj.name + ' / revealed state', width, max(rows, 1), alpha=True)
        texture = next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE')
        texture.image = new_image
        next(n for n in material.node_tree.nodes if n.type == 'UVMAP').uv_map = OWNED_UV
        pixels = np.zeros((max(rows, 1), width, 4), dtype=np.uint8)
        packed = np.ones((max(rows, 1), width), dtype=bool)
        old_rows = 0
    else:
        owned_slots = [i for i, m in enumerate(mesh.materials) if m and m.get('source_ownership_bake')]
        if len(owned_slots) != 1:
            raise ValueError(f'{obj.name}: expected one owned-exterior material slot')
        for polygon in mesh.polygons:
            polygon.material_index = owned_slots[0]  # cut faces inherit the cutter's slot 0
        slot, material, new_image, uv = isolate_material(obj, 'revealed state')
        if uv != OWNED_UV:
            raise ValueError(f'{obj.name}: owned material binds {uv}, expected {OWNED_UV}')
        width, height = new_image.size
        areas = uv_area_texels(obj, OWNED_UV, (width, height))
        bad = [i for i, a in enumerate(areas) if a < 0.05]
        layer = mesh.uv_layers[OWNED_UV]
        for loop in layer.data:
            loop.uv = (loop.uv[0] * width, loop.uv[1] * height)
        extra = pack_faces(obj, bad, OWNED_UV, width, height) if bad else 0
        old = G.read_image(new_image)
        rows = height + extra
        pixels = np.zeros((rows, width, 4), dtype=np.uint8)
        pixels[:height] = old
        packed = np.zeros((rows, width), dtype=bool)
        packed[height:] = True
        old_rows = height
        if extra:
            new_image.scale(width, rows)
    layer = mesh.uv_layers[OWNED_UV]
    width, rows = new_image.size
    for loop in layer.data:
        loop.uv = (loop.uv[0] / width, loop.uv[1] / rows)
    # New island texels start neutral (shaded by their face normal), alpha 255 = unknown.
    rec = record(obj)
    unknown = np.zeros(pixels.shape[:2], dtype=bool)
    slot_uv = G.slot_uvs(obj, OWNED_UV)
    for face, ty, tx, positions, normals, inner in G.islands(rec, slot_uv, (width, rows), lambda group: True):
        fresh = packed[ty, tx]
        if not fresh.any():
            continue
        shade = np.rint(G.neutral_value(normals[fresh]) * 255).astype(np.uint8)
        pixels[ty[fresh], tx[fresh], :3] = shade[:, None]
        pixels[ty[fresh], tx[fresh], 3] = 255
        unknown[ty[fresh], tx[fresh]] = True
    G.write_image(new_image, pixels)
    obj['revealed_state_atlas'] = json.dumps({'appended_rows': rows - old_rows, 'width': width,
                                              'original_rows': old_rows}, sort_keys=True)
    return unknown


def native_order(patch):
    layers = json.loads((WORK / 'source-states/layers.json').read_text())
    return [row['id'] for row in layers['patches']].index(patch)


def remove_states(assets):
    """Delete the state objects (and their isolated materials/images) of the given assets and
    clear the reveal lists their build added to approved objects. Returns what was removed."""
    removed, cleared = [], []
    victims = [o for o in working_meshes() if o.get('asset_group') in assets and o.get('state_recipe')]
    materials = {m for o in victims for m in o.data.materials if m}
    for obj in victims:
        mesh = obj.data
        removed.append(obj.name)
        bpy.data.objects.remove(obj, do_unlink=True)
        if mesh.users == 0:
            bpy.data.meshes.remove(mesh)
    for material in materials:
        if material.users == 0:
            images = [n.image for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image] \
                if material.node_tree else []
            bpy.data.materials.remove(material)
            for image in images:
                if image.users == 0:
                    bpy.data.images.remove(image)
    for obj in working_meshes():
        if obj.get('asset_group') in assets:
            for key in ('reveal_hide_when_applied', 'reveal_show_when_applied'):
                if key in obj:
                    del obj[key]
                    cleared.append(obj.name)
    if not removed:
        raise ValueError('No state objects found to replace for ' + ', '.join(sorted(assets)))
    return {'assets': sorted(assets), 'removed_objects': sorted(removed), 'cleared_reveal_lists': sorted(set(cleared))}


def appearance_copy(obj, state):
    copy = obj.copy()
    copy.data = obj.data.copy()
    copy.name = f'{obj.name} :: revealed {state}'
    copy.data.name = copy.name
    for collection in obj.users_collection:
        collection.objects.link(copy)
    for key in ('reveal_hide_when_applied', 'reveal_show_when_applied'):
        if key in copy:
            del copy[key]
    isolate_material(copy, 'revealed ' + state)
    copy['state_recipe'] = TAG
    copy['state_appearance_of'] = obj.name
    copy['projection_component'] = f'revealed-{state}-appearance'
    copy.hide_render = True
    return copy


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument('--stage-in', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--dry-run', action='store_true', help='count texels, do not save')
    parser.add_argument('--replace-assets', nargs='+', metavar='ASSET',
                        help='Stage-in already carries baked states: remove only these assets\' state objects '
                             '(state_recipe), clear their approved objects\' reveal lists, and rebake just them; '
                             'every other object stays byte-identical')
    args = parser.parse_args(argv)
    output = args.output.resolve()
    if output.exists():
        raise FileExistsError(output)
    render_slots.acquire()
    worker = (args.stage_in / 'worker.blend').resolve(strict=True)
    bpy.ops.wm.open_mainfile(filepath=str(worker))
    tooling = json.loads((WORK / 'tooling/current.json').read_text())['directory']
    sys.path.insert(0, tooling)
    from occlusion_constraints import SourceMaskConstraints

    report = {'version': 1, 'recipe': TAG, 'stage_in': str(worker), 'stage_in_sha256': sha(worker),
              'assets': {}, 'states': {}, 'objects': {}, 'conflicts': []}
    specs = {}
    for spec_path in sorted((REVIEW / 'spec').glob('*.json')):
        if spec_path.name.endswith('.review.json'):
            continue
        spec = json.loads(spec_path.read_text())
        specs[spec['asset_id']] = (spec, spec_path)
    handoff = json.loads((REVIEW / 'HANDOFF.json').read_text())
    if set(specs) != set(handoff['assets']):
        raise ValueError('State specs differ from the approved handoff assets')
    if args.replace_assets:
        if set(args.replace_assets) - set(specs):
            raise ValueError('Unknown replacement assets')
        specs = {a: specs[a] for a in args.replace_assets}
        report['replaced'] = remove_states(set(args.replace_assets))
    elif any(o.get('state_recipe') for o in working_meshes()):
        raise ValueError('Stage-in already carries state objects; pass --replace-assets')
    digests_before = {}
    before = {o.name: object_fingerprint(o, digests_before) for o in working_meshes()}
    for asset, (spec, spec_path) in specs.items():
        geometry = RG.build(spec, spec_path, include_context=False)
        approved = json.loads((REVIEW / 'models' / asset / 'state-digests.json').read_text())['digests']
        rebuilt = {bpy.data.objects[name].get('projection_component') + '|' + bpy.data.objects[name]['source_node']: d
                   for name, d in geometry['world_digests'].items()}
        if rebuilt != approved:
            raise ValueError(f'{asset}: rebuilt state geometry differs from the approved state model')
        report['assets'][asset] = {'spec_sha256': sha(spec_path), 'state_objects': sorted(geometry['world_digests']),
                                   'approved_state_model_sha256': handoff['assets'][asset]['state_model_sha256']}
    built = {name for row in report['assets'].values() for name in row['state_objects']}
    state_objects = [bpy.data.objects[name] for name in sorted(built)]
    unknown = {}
    for obj in state_objects:
        template = bpy.data.objects[obj['state_variant_of']] if obj.get('state_variant_of') else next(
            o for o in working_meshes() if o.get('asset_group') == obj['asset_group']
            and o.get('source_node') == obj['source_node'] and not o.get('state_recipe'))
        unknown[obj.name] = prepare_state_object(obj, template)
    sources = json.loads((REVIEW / 'sources/manifest.json').read_text())['states']
    wanted = {state for spec, _ in specs.values() for state in spec['states']}
    primary = sorted(s for s, row in sources.items() if len(row['applied_patches']) == 1 and s in wanted)
    combos = sorted(s for s, row in sources.items() if len(row['applied_patches']) > 1 and s in wanted)
    assets_of = {}
    for asset, (spec, _) in specs.items():
        for state in spec['states']:
            assets_of.setdefault(state, []).append(asset)
    change = {s: np.asarray(__import__('PIL.Image', fromlist=['Image']).open(row['change_region'])) > 0
              for s, row in sources.items()}
    writes = {}  # (object name, state) -> (rows, cols, rgb) pending, applied after conflicts resolve
    for state in primary + combos:
        row = sources[state]
        if sha(row['image']) != row['sha256']:
            raise ValueError('State source changed: ' + state)
        applied = set(row['applied_patches'])
        exclusive = set(row['applied_patches'][1:]) if state in combos else None
        from PIL import Image
        source = np.asarray(Image.open(row['image']).convert('RGB'))
        height, width = source.shape[:2]
        x0, y0, x1, y1 = row['change_bbox']
        window = (x0 - 8, y0 - 8, x1 + 8, y1 + 8)
        visible_objects = [o for o in working_meshes() if shown(o, applied)
                           and not o.get('state_appearance_of')]
        zbuffer = ZBuffer(visible_objects, window, (width, height))
        stats = {'objects': {}}
        for asset in assets_of.get(state, []):
            masks = SourceMaskConstraints(REVIEW / 'masks' / asset / 'source-masks.json', 'state-' + state,
                                          row['sha256'], (width, height))
            if not masks.active:
                raise ValueError(f'{asset}: no reviewed state mask for {state}')
            for obj in visible_objects:
                if obj.get('asset_group') != asset:
                    continue
                if exclusive is not None and not (set(obj.get('reveal_show_when_applied', [])) & exclusive):
                    continue
                rec = record(obj)
                slot, material, texture, image, uv = owned_binding(obj)
                size = image.size
                slot_uv = G.slot_uvs(obj, uv)
                allowed_masks = masks.for_object(obj)
                hit_rows, hit_cols, hit_rgb = [], [], []
                for face, ty, tx, positions, normals, inner in G.islands(rec, slot_uv, size, lambda group: True):
                    facing = normals @ G.TOWARD > MIN_FACING
                    if not facing.any():
                        continue
                    x, v, _ = G.screen(positions)
                    sx = np.floor(x).astype(np.int64)
                    sv = np.floor(v).astype(np.int64)
                    inside = facing & (sx >= 0) & (sx < width) & (sv >= 0) & (sv < height)
                    inside[inside] &= change[state][sv[inside], sx[inside]]
                    if not inside.any():
                        continue
                    inside[inside] &= zbuffer.visible(positions[inside])
                    if allowed_masks is not None and inside.any():
                        inside[inside] &= masks.allowed(allowed_masks, sx[inside], height - 1 - sv[inside])
                    if inside.any():
                        hit_rows.append(ty[inside]); hit_cols.append(tx[inside])
                        hit_rgb.append(source[sv[inside], sx[inside]])
                if hit_rows:
                    rows_, cols_ = np.concatenate(hit_rows), np.concatenate(hit_cols)
                    writes[(obj.name, state)] = (rows_, cols_, np.concatenate(hit_rgb))
                    stats['objects'][obj.name] = int(len(rows_))
        report['states'][state] = {'applied_patches': sorted(applied), 'source_sha256': row['sha256'],
                                   'window': window, **stats}
        print(f'Revealed bake: {state} {sum(stats["objects"].values())} texels on {len(stats["objects"])} objects',
              flush=True)
    # Appearance copies for approved objects; supersession between states writing one object.
    by_object = {}
    for name, state in writes:
        by_object.setdefault(name, []).append(state)
    copies = {}
    for name, states in sorted(by_object.items()):
        obj = bpy.data.objects[name]
        if obj.get('state_recipe'):
            continue
        base_hide = list(obj.get('reveal_hide_when_applied', []))
        for state in states:
            copy = appearance_copy(obj, state)
            patch = sources[state]['applied_patches'][-1]
            copy['reveal_show_when_applied'] = [patch]
            hide = set(base_hide)
            for other in states:
                if other == state:
                    continue
                mine, theirs = change[state], change[other]
                other_patch = sources[other]['applied_patches'][-1]
                # With both patches applied the game composites them in native patch order,
                # so the later patch's art is on top: its copy supersedes this one.
                if native_order(other_patch) > native_order(patch):
                    hide.add(other_patch)
                    report['conflicts'].append({'object': name, 'loser': state, 'winner': other,
                                                'rule': 'later native patch on top',
                                                'lost_texels': int(len(writes[(name, state)][0]))})
            if hide:
                copy['reveal_hide_when_applied'] = sorted(hide)
            RG.mark(obj, 'reveal_hide_when_applied', [patch])
            copies[(name, state)] = copy
    for (name, state), (rows_, cols_, rgb) in writes.items():
        target = copies.get((name, state), bpy.data.objects[name])
        if not target.get('state_recipe'):
            raise RuntimeError('Revealed texels would be written to an approved object: ' + name)
        slot, material, texture, image, uv = owned_binding(target)
        pixels = G.read_image(image)
        pixels[rows_, cols_, :3] = rgb
        pixels[rows_, cols_, 3] = 255
        G.write_image(image, pixels)
        if name in unknown:
            unknown[name][rows_, cols_] = False
        report['objects'].setdefault(target.name, {})[state] = int(len(rows_))
    for name, mask in unknown.items():
        report['objects'].setdefault(name, {})['unknown_texels'] = int(mask.sum())
    # State objects are exported visible: the exporter requires one default visibility per
    # canonical part, and the editor's PatchDisplay hides every object whose
    # reveal_show_when_applied patch is not applied (all patches start covered). Covered-state
    # audits must therefore select objects by these properties, not by hide_render.
    for obj in working_meshes():
        if obj.get('state_recipe'):
            obj.hide_render = False
            obj.hide_viewport = False
            if 'reveal_show_when_applied' not in obj:
                raise RuntimeError('State object without a show patch would be visible while covered: ' + obj.name)
    digests_after = {}
    after = {name: object_fingerprint(bpy.data.objects[name], digests_after) for name in before}
    drift = sorted(name for name in before if before[name] != after[name])
    if drift:
        raise ValueError('Stage-in objects changed: ' + ', '.join(drift[:20]))
    report['stage_in_objects_unchanged'] = len(before)
    report['unknown_texels_total'] = sum(v.get('unknown_texels', 0) for v in report['objects'].values())
    report['verification'] = verify(sources, primary + combos, change)
    output.mkdir(parents=True)
    if not args.dry_run:
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(output / 'worker.blend'), compress=False)
        report['worker_sha256'] = sha(output / 'worker.blend')
    (output / 'revealed-bake.json').write_text(json.dumps(report, indent=2) + '\n')
    print('REVEALED-BAKE', json.dumps({'unknown': report['unknown_texels_total'],
                                        'conflicts': len(report['conflicts']),
                                        'verification': {s: v['mean_abs_error'] for s, v in report['verification'].items()}}))


def verify(sources, states, change):
    """Source-camera colour of each state's visible set against its source, in the change region."""
    from PIL import Image
    result = {}
    for state in states:
        row = sources[state]
        applied = set(row['applied_patches'])
        source = np.asarray(Image.open(row['image']).convert('RGB')).astype(np.float64)
        height, width = source.shape[:2]
        objects = [o for o in working_meshes() if (shown(o, applied) if not o.get('state_recipe') else
                   ('reveal_show_when_applied' in o and shown(o, applied)))]
        ys, xs = np.nonzero(change[state])
        errors, gray = [], 0
        scene = bpy.context.scene
        depsgraph = bpy.context.evaluated_depsgraph_get()
        hidden = {}
        for o in working_meshes():
            hidden[o.name] = o.hide_viewport
            o.hide_viewport = o not in objects
        depsgraph.update()
        from mathutils import Vector
        toward = Vector(G.TOWARD)
        step = max(1, len(xs) // 4000)
        cache = {}
        for x, y in zip(xs[::step], ys[::step]):
            origin = Vector((x + .5, -(y + .5) / G.SIN, 0)) + toward * 20000
            hit, location, normal, face, obj, _ = scene.ray_cast(depsgraph, origin, -toward)
            if not hit or obj.get('asset_group') is None:
                continue
            try:
                slot, material, texture, image, uv = owned_binding(obj)
            except ValueError:
                continue
            if image.name not in cache:
                cache[image.name] = G.read_image(image)
            pixels = cache[image.name]
            polygon = obj.data.polygons[face]
            from mathutils.interpolate import poly_3d_calc
            points = [obj.matrix_world @ obj.data.vertices[v].co for v in polygon.vertices]
            weights = poly_3d_calc(points, location)
            layer = obj.data.uv_layers[uv]
            u = sum(layer.data[l].uv[0] * w for l, w in zip(polygon.loop_indices, weights))
            v = sum(layer.data[l].uv[1] * w for l, w in zip(polygon.loop_indices, weights))
            h, w_ = pixels.shape[:2]
            texel = pixels[min(h - 1, int(v * h)), min(w_ - 1, int(u * w_)), :3].astype(np.float64)
            errors.append(np.abs(texel - source[y, x]).mean())
            gray += int(texel[0] == texel[1] == texel[2])
        for o in working_meshes():
            o.hide_viewport = hidden[o.name]
        errors = np.array(errors) if errors else np.zeros(1)
        result[state] = {'sampled_pixels': int(len(errors)), 'mean_abs_error': round(float(errors.mean()), 2),
                         'within_12': round(float((errors <= 12).mean()), 3), 'exact_gray_texels': gray}
    return result


if __name__ == '__main__':
    main(sys.argv[sys.argv.index('--') + 1:])
