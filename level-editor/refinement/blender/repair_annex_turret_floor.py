"""Move the turret's textured cap onto its retained landing without rebuilding walls."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Vector


def render_review(path, center, scale, azimuth=0):
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.device = 'CPU'
    scene.cycles.samples = 8
    scene.render.threads_mode = 'FIXED'
    scene.render.threads = 2
    scene.render.resolution_x = scene.render.resolution_y = 640
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = True
    camera_data = bpy.data.cameras.new('Review camera')
    camera_data.type = 'ORTHO'
    camera_data.ortho_scale = scale
    camera_data.clip_end = 10000
    camera = bpy.data.objects.new('Review camera', camera_data)
    scene.collection.objects.link(camera)
    angle = math.radians(azimuth)
    camera.location = center + Vector((1500 * math.sin(angle), -1500 * math.cos(angle), 1500))
    camera.rotation_euler = (center - camera.location).to_track_quat('-Z', 'Y').to_euler()
    scene.camera = camera
    scene.render.filepath = str(path.resolve())
    bpy.ops.render.render(write_still=True)
    probes = []
    rotation = camera.matrix_world.to_quaternion()
    for x, y in [(320, 125), (320, 134), (300, 140), (320, 137), (320, 145), (340, 145)]:
        origin = camera.location + rotation @ Vector(((x / 640 - 0.5) * scale,
                                                      (0.5 - y / 640) * scale, 0))
        hit, position, normal, face, obj, matrix = scene.ray_cast(
            bpy.context.evaluated_depsgraph_get(), origin, rotation @ Vector((0, 0, -1)))
        if hit:
            polygon = obj.data.polygons[face]
            probes.append({'pixel': [x, y], 'node': obj.get('source_node'), 'face': face,
                           'position': list(position),
                           'vertices': [list(matrix @ obj.data.vertices[i].co) for i in polygon.vertices],
                           'uv': [list(obj.data.uv_layers.active.data[i].uv) for i in polygon.loop_indices]})
    path.with_suffix('.hits.json').write_text(json.dumps(probes, indent=2) + '\n')
    scene.camera = None
    bpy.data.objects.remove(camera, do_unlink=True)
    bpy.data.cameras.remove(camera_data)


def triangles_with_attributes(obj):
    mesh = obj.data
    mesh.calc_loop_triangles()
    assert len(mesh.materials) == 1 and mesh.uv_layers.active
    normal_matrix = obj.matrix_world.to_3x3().inverted().transposed()
    return [{
        'positions': [obj.matrix_world @ mesh.vertices[i].co for i in t.vertices],
        'uv': [mesh.uv_layers.active.data[i].uv.copy() for i in t.loops],
        'normals': [(normal_matrix @ mesh.corner_normals[i].vector).normalized() for i in t.loops],
        'smooth': mesh.polygons[t.polygon_index].use_smooth,
    } for t in mesh.loop_triangles]


def replace_triangles(obj, triangles, material):
    inverse = obj.matrix_world.inverted()
    replacement = bpy.data.meshes.new(obj.data.name + " / receiver repair")
    replacement.from_pydata([inverse @ p for t in triangles for p in t['positions']], [],
                           [(i * 3, i * 3 + 1, i * 3 + 2) for i in range(len(triangles))])
    replacement.materials.append(material)
    uv = replacement.uv_layers.new(name="UVMap")
    for polygon, triangle in zip(replacement.polygons, triangles):
        polygon.use_smooth = triangle['smooth']
        for index, value in zip(polygon.loop_indices, triangle['uv']):
            uv.data[index].uv = value
    normal_matrix = obj.matrix_world.to_3x3().transposed()
    replacement.normals_split_custom_set([(normal_matrix @ n).normalized()
                                         for t in triangles for n in t['normals']])
    replacement.update()
    obj.data = replacement
    # The retained texture is transferred from the existing asset. Old geometry
    # provenance no longer describes this mesh and must not certify the repair.
    for key in list(obj.keys()):
        if key.startswith("reprojection_"):
            del obj[key]


def fit_floor_uv_to_textured_cap(triangles, material):
    """Fit the floor chart inside its existing painted disk, preserving one affine map."""
    images = [n.image for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image]
    assert len(images) == 1
    image = images[0]
    width, height = image.size
    pixels = list(image.pixels)
    uvs = [uv for triangle in triangles for uv in triangle['uv']]
    low = Vector((min(uv.x for uv in uvs), min(uv.y for uv in uvs)))
    high = Vector((max(uv.x for uv in uvs), max(uv.y for uv in uvs)))
    center = (low + high) * 0.5
    samples = [t['uv'][0] * (a / 16) + t['uv'][1] * (b / 16)
               + t['uv'][2] * (1 - (a + b) / 16)
               for t in triangles for a in range(17) for b in range(17 - a)]

    def unpainted(uv):
        x, y = int(uv.x * width), int(uv.y * height)
        assert 0 <= x < width and 0 <= y < height
        rgb = pixels[(y * width + x) * 4: (y * width + x) * 4 + 3]
        return max(rgb) - min(rgb) < 0.005 and any(
            abs(rgb[0] - value / 255) < 0.005 for value in (41, 74))

    before = sum(unpainted(uv) for uv in samples)
    scale = 1.0
    while any(unpainted(center + (uv - center) * scale) for uv in samples):
        scale -= 0.01
        assert scale >= 0.5, 'Cap chart cannot fit a sufficiently large painted region'
    for triangle in triangles:
        triangle['uv'] = [center + (uv - center) * scale for uv in triangle['uv']]
    return {'center': list(center), 'scale': scale, 'samples': len(samples),
            'unpaintedBefore': before, 'unpaintedAfter': 0,
            'neutralPaddingRgb': [41, 74],
            'scope': 'single-affine-floor-UV-remap-existing-atlas-pixels-unchanged'}


def fit_inner_wall_uv(triangles, cap, material, cosine, floor_height):
    image = next(n.image for n in material.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image)
    width, height = image.size
    pixels = list(image.pixels)
    center = sum((p for t in cap for p in t['positions']), Vector()) / (3 * len(cap))
    weights = [(a / 8, b / 8, 1 - (a + b) / 8)
               for a in range(9) for b in range(9 - a)]

    def neutral(uv):
        offset = (int(uv.y * height) * width + int(uv.x * width)) * 4
        rgb = pixels[offset:offset + 3]
        return max(rgb) - min(rgb) < 0.01 and 0.04 < rgb[0] < 0.4

    changed, unresolved = [], []
    for index, triangle in enumerate(triangles):
        positions = triangle['positions']
        heights = [p.z * cosine for p in positions]
        if min(heights) < floor_height or max(heights) > 510:
            continue
        normal = (positions[1] - positions[0]).cross(positions[2] - positions[0]).normalized()
        radial = sum(positions, Vector()) / 3 - center
        radial.z = 0
        if normal.dot(radial) >= 0:
            continue
        samples = [sum((uv * w for uv, w in zip(triangle['uv'], weights_)), Vector((0, 0)))
                   for weights_ in weights]
        if not any(neutral(uv) for uv in samples):
            continue
        donors = [uv for uv in samples if not neutral(uv)]
        found = None
        for scale in (0.9, 0.75, 0.5, 0.25, 0.125):
            for donor in donors:
                if all(not neutral(donor + (uv - donor) * scale) for uv in samples):
                    found = donor, scale
                    break
            if found:
                break
        if not found:
            unresolved.append(index)
            continue
        donor, scale = found
        triangle['uv'] = [donor + (uv - donor) * scale for uv in triangle['uv']]
        changed.append({'triangle': index, 'scale': scale, 'center': list(donor)})
    return {'changed': changed, 'unresolved': unresolved,
            'scope': 'inward-upper-wall-UV-only-existing-atlas-no-geometry-edits'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--fit-floor-uv', action='store_true')
    parser.add_argument('--fit-inner-wall-uv', action='store_true')
    parser.add_argument('--review-all-sides', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    scene = json.loads((args.library / 'scenes/lincoln.rhlos-map.json').read_bytes())
    source = next(r for r in scene['assetSources'] if r['id'] == 'lincoln-keep-annex-round-turret')
    for field in ('descriptor', 'model'):
        assert hashlib.sha256((args.library / source[field]).read_bytes()).hexdigest() == source[field + '_sha256']
    descriptor = json.loads((args.library / source['descriptor']).read_bytes())
    args.output.mkdir(parents=True, exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str((args.library / source['model']).resolve()))
    bpy.context.scene.name = descriptor['model_scene']
    bounds = [o.matrix_world @ Vector(p) for o in bpy.context.scene.objects
              if o.type == 'MESH' for p in o.bound_box]
    low = Vector(tuple(min(p[i] for p in bounds) for i in range(3)))
    high = Vector(tuple(max(p[i] for p in bounds) for i in range(3)))
    center, scale = (low + high) * 0.5, (high - low).length * 1.15
    render_review(args.output / 'before.png', center, scale)
    angle = math.radians(scene['camera']['elevation_deg'])
    def part_object(node):
        objects = [o for o in bpy.context.scene.objects if o.type == 'MESH' and o.get('source_node') == node]
        assert len(objects) == 1
        return objects[0]

    wall, floor = part_object('building-206'), part_object('building-228')
    material = wall.data.materials[0]
    records = triangles_with_attributes(wall)
    cap = [t for t in records if all(abs(p.z * math.cos(angle) - 486) < 0.001 for p in t['positions'])]
    assert cap and len(cap) < len(records)
    retained = [t for t in records if t not in cap]
    floor_uv = fit_floor_uv_to_textured_cap(cap, material) if args.fit_floor_uv else None
    points = next(p for p in descriptor['parts'] if p['node'] == 'building-228')['obstacle_local_game']['points']
    height = points[0]['z_top']
    assert all(abs(p['z_top'] - height) < 1e-6 for p in points)
    wall_uv = fit_inner_wall_uv(retained, cap, material, math.cos(angle), height) if args.fit_inner_wall_uv else None
    for triangle in cap:
        for position in triangle['positions']:
            position.z = height / math.cos(angle)
    replace_triangles(wall, retained, material)
    replace_triangles(floor, cap, material)
    output = args.output / 'model.glb'
    bpy.ops.export_scene.gltf(filepath=str(output.resolve()), export_format='GLB',
                              export_extras=True, export_yup=True, export_animations=False)
    render_review(args.output / 'after.png', center, scale)
    if args.review_all_sides:
        for azimuth in (90, 180, 270):
            render_review(args.output / f'after-{azimuth}.png', center, scale, azimuth)
    (args.output / 'review.json').write_text(json.dumps({
        'scope': 'unpublished-mesh-receiver-repair-requires-render-and-gameplay-review',
        'source': source, 'outputSha256': hashlib.sha256(output.read_bytes()).hexdigest(),
        'retainedWallTriangles': len(retained), 'loweredCapTriangles': len(cap),
        'beforeCapHeight': 486, 'afterCapHeight': height,
        'floorUv': floor_uv,
        'innerWallUv': wall_uv,
        'appearance': 'Positions, corner normals and materials retained; floor and inward upper-wall UV changes recorded separately. Exterior UVs and atlas pixels unchanged.',
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
