"""Inspect independently saved western rocks and foreground shrubs together."""
import argparse
import json
import math
import sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).parent))
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from catalog import OUT, scenery_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, RAY


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('shrub_worker', type=Path)
    parser.add_argument('shrub_sha256')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    rock_worker = scenery_workspace('croisement02-west-rock-outcrop')
    workers = [rock_worker, args.shrub_worker.resolve()]
    hashes = [sha(w / 'model.blend') for w in workers]
    if hashes[1] != args.shrub_sha256:
        raise ValueError('Shrub candidate changed since independent review')
    destination = OUT / 'west-rock-joint-review' / (hashes[0][:8] + '-' + hashes[1][:8])
    if destination.exists():
        attempt = 2
        while destination.with_name(destination.name + f'-{attempt}').exists():
            attempt += 1
        destination = destination.with_name(destination.name + f'-{attempt}')
    destination.mkdir(parents=True, exist_ok=False)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(rock_worker / 'model.blend'))
        scene = bpy.data.scenes.new('Western rocks and foreground shrubs')
        bpy.context.window.scene = scene
        meshes = []
        for original in list(bpy.data.collections['Croisement02 Working'].all_objects):
            if original.type != 'MESH' or original.get('asset_group') != rock_worker.name:
                continue
            obj = original.copy()
            transform = original.matrix_world.copy()
            obj.parent = None
            obj.matrix_world = transform
            scene.collection.objects.link(obj)
            obj.hide_render = False
            meshes.append(obj)
        with bpy.data.libraries.load(str(workers[1] / 'model.blend'), link=False) as (source, loaded):
            if 'Croisement02 Working' not in source.collections:
                raise ValueError('Shrub worker has no isolated Working collection')
            loaded.collections = ['Croisement02 Working']
        shrubs = [o for o in loaded.collections[0].all_objects
                  if o.type == 'MESH' and o.get('asset_group') == workers[1].name]
        if not shrubs:
            raise ValueError('No authored shrub meshes found')
        for obj in shrubs:
            transform = obj.matrix_world.copy()
            obj.parent = None
            obj.matrix_world = transform
            scene.collection.objects.link(obj)
            obj.hide_render = False
            meshes.append(obj)
        scene.world = bpy.data.worlds.new('Joint review neutral environment')
        scene.world.color = (.12, .12, .12)
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 16
        scene.cycles.transparent_max_bounces = 64
        scene.render.image_settings.file_format = 'PNG'
        scene.render.image_settings.color_mode = 'RGBA'
        scene.render.film_transparent = True
        scene.view_settings.view_transform = 'Standard'
        scene.view_settings.look = 'None'
        scene.render.resolution_percentage = 100
        points = [o.matrix_world @ v.co for o in meshes for v in o.data.vertices]
        lo = Vector([min(p[i] for p in points) for i in range(3)])
        hi = Vector([max(p[i] for p in points) for i in range(3)])
        center = (lo + hi) / 2
        light_data = bpy.data.lights.new('Joint review sun', 'SUN')
        light_data.energy = 2
        light = bpy.data.objects.new(light_data.name, light_data)
        scene.collection.objects.link(light)
        light.rotation_euler = (math.radians(28), math.radians(-25), math.radians(-30))
        camera_data = bpy.data.cameras.new('Joint review camera')
        camera_data.type = 'ORTHO'
        camera_data.sensor_fit = 'HORIZONTAL'
        camera_data.clip_end = 20000
        camera = bpy.data.objects.new(camera_data.name, camera_data)
        scene.collection.objects.link(camera)
        scene.camera = camera
        cameras = []

        def render(name, target, direction, scale, width, height):
            camera.location = target + direction * 5000
            camera.rotation_euler = (target - camera.location).to_track_quat('-Z', 'Y').to_euler()
            camera_data.ortho_scale = scale
            bpy.context.view_layer.update()
            scene.render.resolution_x = width
            scene.render.resolution_y = height
            scene.render.filepath = str(destination / name)
            bpy.ops.render.render(write_still=True, scene=scene.name)
            cameras.append(dict(image=name, matrix=[list(row) for row in camera.matrix_world], ortho_scale=scale))

        crop = (-100, 230, 310, 510)
        left, top, right, bottom = crop
        target = Vector(((left + right) / 2, -(top + bottom) / 2 / SIN, 0))
        render('source-view.png', target, RAY, right - left, right - left, bottom - top)
        source = Image.open(OUT / 'animation-references/composite-frame-0.png').convert('RGB').crop(crop)
        source.save(destination / 'source.png')
        actual = Image.open(destination / 'source-view.png').convert('RGBA')
        board = Image.new('RGB', (source.width * 2, source.height + 24), '#454545')
        board.paste(source, (0, 24))
        board.paste(actual, (source.width, 24), actual)
        draw = ImageDraw.Draw(board)
        draw.text((4, 4), 'Original source; left padding is outside map', fill='white')
        draw.text((source.width + 4, 4), 'Saved rock and shrub materials together', fill='white')
        board.resize((board.width * 2, board.height * 2), Image.Resampling.NEAREST).save(destination / 'source-comparison.png')
        scale = max((hi - lo).length * 1.10, 100)
        for i in range(8):
            angle = 2 * math.pi * i / 8
            direction = Vector((math.sin(angle) * math.cos(.55), -math.cos(angle) * math.cos(.55), math.sin(.55)))
            render(f'view-{i}.png', center, direction, scale, 512, 384)
        board = Image.new('RGB', (2048, 768), '#454545')
        for i in range(8):
            image = Image.open(destination / f'view-{i}.png').convert('RGBA')
            board.paste(image, ((i % 4) * 512, (i // 4) * 384), image)
        board.save(destination / 'sheet.png')
        # A diagnostic plane locates world Z=0; it is not a terrain candidate.
        ground_mesh = bpy.data.meshes.new('Diagnostic ground datum')
        pad = 100
        ground_mesh.from_pydata([(lo.x-pad, lo.y-pad, 0), (hi.x+pad, lo.y-pad, 0),
                                 (hi.x+pad, hi.y+pad, 0), (lo.x-pad, hi.y+pad, 0)], [], [(0, 1, 2, 3)])
        ground = bpy.data.objects.new(ground_mesh.name, ground_mesh)
        scene.collection.objects.link(ground)
        ground_material = bpy.data.materials.new('Diagnostic datum only')
        ground_material.diffuse_color = (.30, .22, .15, 1)
        ground.data.materials.append(ground_material)
        contact = Image.new('RGB', (2048, 384), '#454545')
        for i in range(4):
            angle = 2 * math.pi * i / 4
            direction = Vector((math.sin(angle) * math.cos(.25), -math.cos(angle) * math.cos(.25), math.sin(.25)))
            render(f'contact-{i}.png', center, direction, scale, 512, 384)
            image = Image.open(destination / f'contact-{i}.png').convert('RGBA')
            contact.paste(image, (i * 512, 0), image)
        contact.save(destination / 'contact-sheet.png')
        for worker, expected in zip(workers, hashes):
            if sha(worker / 'model.blend') != expected:
                raise ValueError('Worker changed during joint render')
        write_json(destination / 'evidence.json', dict(workers=[dict(path=str(w), model_sha256=h) for w, h in zip(workers, hashes)],
            meshes=[dict(name=o.name, asset_group=o.get('asset_group'),
                         minimum_world_z=min((o.matrix_world @ v.co).z for v in o.data.vertices),
                         maximum_world_z=max((o.matrix_world @ v.co).z for v in o.data.vertices)) for o in meshes], cameras=cameras,
            source_crop=list(crop), sheet_sha256=sha(destination / 'sheet.png'), source_comparison_sha256=sha(destination / 'source-comparison.png'),
            status='Rendered candidate pair; requires visual review. Terrain and other plants are absent.'))
        print(destination)
    finally:
        release()


if __name__ == '__main__':
    main()
