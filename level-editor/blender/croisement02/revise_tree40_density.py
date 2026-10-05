"""Private inferred-cluster thinning with exact observed-crown preservation."""
import argparse
from array import array
import json
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from bake_texture_candidate import snapshot, pixels, sha, require, acquire, release
from catalog import OUT
from evidence_io import write_json
from refinement_review import _tree, _tile
from render_multiview_asset import render
from tree_geometry import SIN, RAY


def coverage(obj):
    tree, _, _ = _tree([obj])
    result = np.zeros((152, 226), bool)
    ray = Vector(RAY)
    for yy in range(152):
        for xx in range(226):
            point = Vector((1679 + xx + .5, -(319 + yy + .5) / SIN, 0)) + ray * 10000
            result[yy, xx] = tree.ray_cast(point, -ray)[0] is not None
    return result


def visible_bounds(obj):
    """Measure occupied texel centers, excluding fully transparent card geometry."""
    low, high = np.full(3, np.inf), np.full(3, -np.inf)
    for slot, material in enumerate(obj.data.materials):
        texture = next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image)
        rgba = pixels(texture.image)
        height, width = rgba.shape[:2]
        uv = obj.data.uv_layers[texture.inputs['Vector'].links[0].from_node.uv_map]
        for face in obj.data.polygons:
            if face.material_index != slot:
                continue
            require(len(face.vertices) == 3, 'Expected triangular crown')
            coords = np.asarray([uv.data[i].uv[:] for i in face.loop_indices]) * [width, height]
            left, bottom = np.maximum(np.floor(coords.min(axis=0)).astype(int), 0)
            right, top = np.minimum(np.ceil(coords.max(axis=0)).astype(int), [width, height])
            yy, xx = np.mgrid[bottom:top, left:right]
            points = np.stack((xx.ravel() + .5, yy.ravel() + .5), axis=1)
            basis = np.stack((coords[1] - coords[0], coords[2] - coords[0]), axis=1)
            if abs(np.linalg.det(basis)) < 1e-8:
                continue
            bc = (points - coords[0]) @ np.linalg.inv(basis).T
            inside = (bc.min(axis=1) >= -1e-6) & (bc.sum(axis=1) <= 1 + 1e-6)
            inside &= rgba[yy.ravel(), xx.ravel(), 3] >= .5
            if not inside.any():
                continue
            weights = np.column_stack((1 - bc[inside].sum(axis=1), bc[inside]))
            world = weights @ np.asarray([obj.matrix_world @ obj.data.vertices[v].co for v in face.vertices])
            low = np.minimum(low, world.min(axis=0))
            high = np.maximum(high, world.max(axis=0))
    require(np.isfinite(low).all(), 'No occupied physical foliage texels')
    return dict(minimum=low.tolist(), maximum=high.tolist(), extent=(high - low).tolist(),
                method='World-space bounds of occupied physical atlas texel centers')


def main(output, retention, belt_retention=None, lobed_cards=False, keep_edge_clusters=False):
    require(0 < retention <= 1, 'Retention must be in (0, 1]')
    require(belt_retention is None or 0 < belt_retention <= retention, 'Invalid belt retention')
    require(not output.exists(), 'Use a fresh private candidate')
    experiment = OUT / 'texture-fill-round-1/croisement02-tree-40/complete-native-front-preparation/experiment'
    donor = OUT / 'texture-fill-round-1/croisement02-tree-40/native-synthesis-scoped-v2'
    source = donor / 'candidate-v1/worker.blend'
    approved = experiment / 'approved-model.blend'
    hashes = {str(p): sha(p) for p in [source, approved, donor / 'donor-validation.json']}
    validation = json.loads((donor / 'candidate-v1/validation.json').read_text())
    require(validation['model_sha256'] == sha(source), 'Scoped texture candidate changed')
    require(json.loads((donor / 'donor-validation.json').read_text())['source_owned_samples'] == 9703,
            'Wrong individual donor authority')
    manifest = json.loads((experiment / 'views.json').read_text())
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source))
        scene = bpy.data.scenes[manifest['scene_name']]
        bpy.context.window.scene = scene
        crown = scene.objects['Centraleast Tree 40 / Crown']
        crown_name = crown.name
        before = snapshot(scene, {crown.name})
        native_before = coverage(crown)
        material = crown.data.materials[3]
        require('inferred outer leaf tiles' in material.name, 'Unexpected outer material')
        texture = next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image)
        rgba = pixels(texture.image).copy()
        original = rgba.copy()
        uv_name = texture.inputs['Vector'].links[0].from_node.uv_map
        uv = crown.data.uv_layers[uv_name]
        faces = [f for f in crown.data.polygons if f.material_index == 3]
        require(len(faces) % 6 == 0, 'Outer crossed clusters must have six triangles')
        rng = np.random.default_rng(240402)
        removed = 0
        kept = 0
        for start in range(0, len(faces), 6):
            cluster = faces[start:start + 6]
            coordinates = [crown.matrix_world @ crown.data.vertices[v].co for f in cluster for v in f.vertices]
            require(min(v.x for v in coordinates) >= 1791.99, 'Outer cluster crossed protected map edge')
            distance = np.mean([v.x for v in coordinates]) - 1792
            probability = retention
            if belt_retention is not None:
                # Thinner branch groups through the previously solid middle
                # belt, with a gradual transition to the outer leaf clumps.
                probability -= (retention - belt_retention) * np.exp(-((distance - 50) / 25) ** 2)
            retain = rng.random() < probability
            at_edge = min(v.x for v in coordinates) <= 1792.02
            if keep_edge_clusters and at_edge:
                retain = True
            if retain:
                kept += 1
                if not lobed_cards or (keep_edge_clusters and at_edge):
                    continue
            else:
                removed += 1
            for face in cluster[::2]:
                # Each crossed card has a private tile shared by its two triangles.
                both = [face, crown.data.polygons[face.index + 1]]
                coords = np.asarray([uv.data[i].uv[:] for f in both for i in f.loop_indices])
                low = np.rint(coords.min(axis=0) * [rgba.shape[1], rgba.shape[0]]).astype(int)
                high = np.rint(coords.max(axis=0) * [rgba.shape[1], rgba.shape[0]]).astype(int)
                require(tuple(high - low) == (24, 24), 'Unexpected inferred card tile')
                region = rgba[low[1]:high[1], low[0]:high[0], 3]
                if retain:
                    yy, xx = np.mgrid[:24, :24] / 12 - 1 + 1 / 24
                    angle = np.arctan2(yy, xx)
                    radius = .93 + .11 * np.sin(5 * angle + start * .13) + .07 * np.sin(9 * angle)
                    region[np.hypot(xx, yy) > radius] = 0
                else:
                    region[:] = 0
        require(removed > 0 and kept > 0, 'No meaningful thinning')
        require(np.array_equal(rgba[:, :, :3], original[:, :, :3]), 'RGB changed')
        texture.image.pixels.foreach_set(rgba.ravel())
        texture.image.update()
        texture.image.pack()
        after = snapshot(scene, {crown.name})
        allowed = crown.name + '/3'
        require(after['geometry'] == before['geometry'], 'Mesh positions or UV changed')
        require(after['outside_appearance'] == before['outside_appearance'], 'Bark or foreign appearance changed')
        for name, state in before['physical_foliage'].items():
            if name == allowed:
                state = dict(state, alpha=after['physical_foliage'][name]['alpha'])
            require(after['physical_foliage'][name] == state, 'Protected foliage changed: ' + name)
        output.mkdir(parents=True)
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 8
        scene.cycles.transparent_max_bounces = 256
        bpy.context.preferences.filepaths.save_version = 0
        model = output / 'worker.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model), compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.data.scenes[manifest['scene_name']]
        crown = scene.objects[crown_name]
        require(snapshot(scene, {crown.name}) == after, 'Reopened candidate changed')
        result = coverage(crown)
        require(np.array_equal(result[:, :113], native_before[:, :113]), 'Native projected coverage changed')
        Image.fromarray(result.astype('uint8') * 255).save(output / 'physical-coverage.png')
        rows = []
        for low, high in [(0, 1), (1, 4), (4, 16), (16, 32), (32, 60), (60, 113)]:
            rows.append(dict(edge_distance=[low, high],
                native=float(result[:, :113][:, ::-1][:, low:high].mean()),
                inferred=float(result[:, 113:][:, low:high].mean()),
                previous_inferred=float(native_before[:, 113:][:, low:high].mean())))
        coords = np.array([crown.matrix_world @ v.co for v in crown.data.vertices])
        width, depth = np.ptp(coords[:, 0]), np.ptp(coords[:, 1])
        require(depth >= width, 'Crown depth is less than width')
        occupied = visible_bounds(crown)
        require(occupied['extent'][1] >= occupied['extent'][0], 'Occupied foliage depth is less than width')
        write_json(output / 'validation.json', dict(status='PASS', model_sha256=sha(model),
            source_hashes=hashes, geometry_unchanged=True, native_rgba_unchanged=True,
            native_projected_coverage_unchanged=True, bark_unchanged=True,
            physical_change='Inferred outer cluster thinning and optional ragged card outlines through private physical alpha tiles',
            retention=retention, belt_retention=belt_retention, lobed_cards=lobed_cards,
            keep_edge_clusters=keep_edge_clusters,
            kept_clusters=kept, removed_clusters=removed,
            width=float(width), depth=float(depth), coverage=rows,
            occupied_physical_bounds=occupied,
            root_review='pending', user_approval=None, publication='none'))
        render(experiment / 'views.json', output / 'actual', width=manifest['tile_size'][0])
        buffers = []
        for i in range(8):
            im = bpy.data.images.load(str(output / 'actual' / f'view-{i}-textured.png'), check_existing=False)
            values = array('f', [0]) * len(im.pixels)
            im.pixels.foreach_get(values)
            buffers.append(values)
            bpy.data.images.remove(im)
        _tile(buffers, *manifest['tile_size'], output / 'actual/textured.png')
        require(all(sha(Path(p)) == h for p, h in hashes.items()), 'Frozen source changed')
    finally:
        release()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--retention', type=float, default=.15)
    parser.add_argument('--belt-retention', type=float)
    parser.add_argument('--lobed-cards', action='store_true')
    parser.add_argument('--keep-edge-clusters', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    main(args.output.resolve(), args.retention, args.belt_retention, args.lobed_cards, args.keep_edge_clusters)
