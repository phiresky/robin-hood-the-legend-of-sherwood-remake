"""Move the turret's textured cap onto its retained landing without rebuilding walls."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Vector


def render_review(path, center, scale):
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
    camera.location = center + Vector((0, -1500, 1500))
    camera.rotation_euler = (center - camera.location).to_track_quat('-Z', 'Y').to_euler()
    scene.camera = camera
    scene.render.filepath = str(path.resolve())
    bpy.ops.render.render(write_still=True)
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--library', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
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
    points = next(p for p in descriptor['parts'] if p['node'] == 'building-228')['obstacle_local_game']['points']
    height = points[0]['z_top']
    assert all(abs(p['z_top'] - height) < 1e-6 for p in points)
    for triangle in cap:
        for position in triangle['positions']:
            position.z = height / math.cos(angle)
    replace_triangles(wall, retained, material)
    replace_triangles(floor, cap, material)
    output = args.output / 'model.glb'
    bpy.ops.export_scene.gltf(filepath=str(output.resolve()), export_format='GLB',
                              export_extras=True, export_yup=True, export_animations=False)
    render_review(args.output / 'after.png', center, scale)
    (args.output / 'review.json').write_text(json.dumps({
        'scope': 'unpublished-mesh-receiver-repair-requires-render-and-gameplay-review',
        'source': source, 'outputSha256': hashlib.sha256(output.read_bytes()).hexdigest(),
        'retainedWallTriangles': len(retained), 'loweredCapTriangles': len(cap),
        'beforeCapHeight': 486, 'afterCapHeight': height,
        'appearance': 'Existing wall/battlement positions, corner normals, UVs and material retained; cap UVs retained exactly on the lowered floor.',
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
