"""Compare packing and density requirements without baking or modifying library models.

blender --background --threads 2 --python-exit-code 1 --python \
  refinement/blender/diagnose_atlas_packing.py -- --output work/atlas-packing
"""
import argparse
import json
import math
from pathlib import Path
import sys

import bpy
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lossy_assets as assets
from render_slots import acquire


def coordinates(objects):
    rows = []
    for obj in objects:
        row = np.empty(len(obj.data.loops) * 2, dtype=np.float32)
        obj.data.uv_layers[assets.NEW_UV].uv.foreach_get('vector', row)
        rows.append(row.reshape(-1, 3, 2))
    return rows


def restore(objects, rows):
    for obj, row in zip(objects, rows):
        obj.data.uv_layers[assets.NEW_UV].uv.foreach_set('vector', row.ravel())


def island_boxes(objects, rows):
    boxes = []
    for obj, faces in zip(objects, rows):
        parent = list(range(len(faces)))
        def find(i):
            while parent[i] != i:
                parent[i] = parent[parent[i]]
                i = parent[i]
            return i
        edges = {}
        for i, (polygon, face) in enumerate(zip(obj.data.polygons, faces)):
            corners = [(int(vertex), tuple(uv)) for vertex, uv in zip(polygon.vertices, face)]
            for j in range(3):
                key = tuple(sorted((corners[j], corners[(j + 1) % 3])))
                if key in edges:
                    parent[find(i)] = find(edges[key])
                else:
                    edges[key] = i
        groups = {}
        for i, face in enumerate(faces):
            low, high = face.min(axis=0), face.max(axis=0)
            key = find(i)
            if key in groups:
                old = groups[key]
                groups[key] = np.minimum(old[0], low), np.maximum(old[1], high)
            else:
                groups[key] = low, high
        boxes.extend((high - low).tolist() for low, high in groups.values())
    return np.asarray(boxes)


def measure(objects, area, targets):
    rows = coordinates(objects)
    faces = np.concatenate(rows).astype(np.float64)
    axes = np.concatenate([assets.face_axes(obj, assets.NEW_UV, 1, 1) for obj in objects])
    positive = area > 1e-12
    weights = area[positive]
    weakest = axes[positive, 0]
    strongest = axes[positive, 1]
    required = targets[positive] / np.maximum(weakest, 1e-12)
    distortion = strongest / np.maximum(weakest, 1e-12)
    delta = faces[:, 1:] - faces[:, :1]
    uv_area = abs(delta[:, 0, 0] * delta[:, 1, 1] - delta[:, 0, 1] * delta[:, 1, 0]) / 2
    boxes = island_boxes(objects, rows)
    mask = Image.new('L', (1024, 1024))
    draw = ImageDraw.Draw(mask)
    for face in faces:
        draw.polygon([tuple(point) for point in face * 1023], fill=255)
    # At a 4096 atlas, two audit pixels approximate an eight-texel bake gutter.
    covered = float(np.asarray(mask).mean() / 255)
    padded = float(np.asarray(mask.filter(ImageFilter.MaxFilter(5))).mean() / 255)
    ideal_pixels = float(np.sum(area * targets**2))
    return {
        'triangles': len(area), 'islands': len(boxes),
        'uv_triangle_area_sum': float(uv_area.sum()),
        'island_aabb_area_sum': float(np.prod(boxes, axis=1).sum()),
        'raster_coverage': covered, 'coverage_with_8px_gutter_at_4096': padded,
        'outside_tile': bool(faces.min() < -1e-6 or faces.max() > 1 + 1e-6),
        'collapsed_uv_triangles': int((uv_area < 1e-12).sum()),
        'required_edge_by_surface_coverage': {str(q): assets.weighted_quantile(required, weights, q) for q in [.5, .9, .95, .99]},
        'axis_distortion_by_surface_coverage': {str(q): assets.weighted_quantile(distortion, weights, q) for q in [.5, .9, .95, .99]},
        'surface_meeting_target_at_4096': float(weights[weakest * 4096 >= targets[positive] - 1e-6].sum() / weights.sum()),
        'ideal_full_surface_edge_no_waste_or_distortion': math.sqrt(ideal_pixels),
        'ideal_full_surface_edge_at_this_occupancy': math.sqrt(ideal_pixels / max(float(uv_area.sum()), 1e-12)),
    }


def diagnose(model, work):
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    doc, binary, _ = assets.read_glb(model)
    images = assets.load_images(doc, binary, work / 'source-images', base=model.parent)
    collection = bpy.data.collections.new('Audit')
    bpy.context.scene.collection.children.link(collection)
    objects, records = assets.build_objects(doc, binary, images, collection, 'source')
    area = np.concatenate([assets.face_geometry(obj, assets.SOURCE_UV)[0] for obj in objects])
    source_axes, nearest = [], []
    for obj, record in zip(objects, records):
        slots = np.empty(len(obj.data.polygons), dtype=np.int32)
        obj.data.polygons.foreach_get('material_index', slots)
        axes = np.zeros((len(slots), 2))
        for slot, (image, *_rest) in enumerate(record['materials']):
            axes[slots == slot] = assets.face_axes(obj, assets.SOURCE_UV, *image.size)[slots == slot]
        source_axes.append(axes)
        nearest.append(np.array([record['materials'][slot][1] == 'Closest' for slot in slots]))
    source_axes = np.concatenate(source_axes)
    nearest = np.concatenate(nearest)
    args = assets.default_settings()
    targets = np.where(nearest, args.nearest_density, np.minimum(args.density, source_axes[:, 1]))
    result = {'model': str(model), 'source_sha256': assets.sha(model), 'settings': assets.settings(args),
              'blender_version': bpy.app.version_string,
              'source_image_sizes': [list(image.size) for image in images.values()],
              'source_pixels': sum(image.size[0] * image.size[1] for image in images.values()), 'variants': {}}
    try:
        size, required, history = assets.unwrap(objects, args, targets)
    except (assets.UnsafeAtlasError, RuntimeError) as error:
        result['default_failure'] = str(error)
        return result
    result['default_decision'] = {'edge': size, 'required': required, 'pack_history': history,
                                 'exceeds_cap': required > args.max_size,
                                 'expansion_limited': assets.atlas_expansion_exceeded(size, [image.size for image in images.values()], args.max_atlas_expansion)}
    result['variants']['default'] = measure(objects, area, targets)
    snapshot = coordinates(objects)
    # Repack the same input UVs with alternate shape and padding settings.
    for name, margin, shape in [('aabb_no_gutter', 0., 'AABB'),
                                ('aabb_8px', 8 / 4096, 'AABB'),
                                ('convex_8px', 8 / 4096, 'CONVEX')]:
        restore(objects, snapshot)
        bpy.ops.object.mode_set(mode='EDIT')
        bpy.ops.mesh.select_all(action='SELECT')
        bpy.ops.uv.select_all(action='SELECT')
        bpy.ops.uv.pack_islands(udim_source='ACTIVE_UDIM', rotate=True, rotate_method='ANY', scale=True,
                               margin_method='FRACTION', margin=margin, shape_method=shape)
        bpy.ops.object.mode_set(mode='OBJECT')
        assets.fit_atlas_tile(objects, margin)
        try:
            assets.check_packed_objects(objects)
            result['variants'][name] = measure(objects, area, targets)
        except assets.UnsafeAtlasError as error:
            result['variants'][name] = {'failure': str(error)}
    # A square destination should not inherit source-material image aspect ratios.
    # Keep geometry, source density targets, and production unwrap settings identical.
    square = bpy.data.images.new('Square destination aspect probe', width=64, height=64)
    replaced = []
    for material in bpy.data.materials:
        if material.node_tree:
            for node in material.node_tree.nodes:
                if node.type == 'TEX_IMAGE':
                    replaced.append((node, node.image))
                    node.image = square
    for obj in objects:
        obj.data.uv_layers.remove(obj.data.uv_layers[assets.NEW_UV])
    try:
        size, required, history = assets.unwrap(objects, args, targets)
        result['variants']['square_material_aspect'] = measure(objects, area, targets)
        result['variants']['square_material_aspect']['pack_history'] = history
    except (assets.UnsafeAtlasError, RuntimeError) as error:
        result['variants']['square_material_aspect'] = {'failure': str(error)}
    finally:
        for node, image in replaced:
            node.image = image
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--assets', nargs='*')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    root = assets.LEVEL_EDITOR / 'library'
    manifest = json.loads((root / 'scenes/Wychford.rhlos-map.json').read_text())
    models = dict.fromkeys(r['model'] for r in manifest['sceneAssets'] + manifest.get('assetSources', []))
    args.output.mkdir(parents=True, exist_ok=True)
    acquire()
    results = []
    for relative in models:
        model = root / relative
        if args.assets and model.parent.name not in args.assets:
            continue
        runtime = model.with_name('lossy.glb')
        if not runtime.exists():
            continue
        from io import BytesIO
        doc, buffers, _ = assets.read_glb(runtime)
        sizes = []
        for image in doc.get('images', []):
            view = doc['bufferViews'][image['bufferView']]
            start = view.get('byteOffset', 0)
            with Image.open(BytesIO(buffers[view.get('buffer', 0)][start:start + view['byteLength']])) as decoded:
                sizes.append(decoded.size)
        if (4096, 4096) not in sizes:
            continue
        work = args.output / model.parent.name
        work.mkdir(exist_ok=True)
        result = diagnose(model, work)
        result['model'] = relative
        (work / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
        results.append(result)
        (args.output / 'summary.json').write_text(json.dumps(results, indent=2) + '\n')
        print('AUDIT', model.parent.name, json.dumps(result.get('default_decision', result.get('default_failure'))), flush=True)
        for name, data in result['variants'].items():
            print(name, json.dumps({k: data[k] for k in ['required_edge_by_surface_coverage', 'raster_coverage', 'axis_distortion_by_surface_coverage', 'failure'] if k in data}), flush=True)


if __name__ == '__main__':
    main()
