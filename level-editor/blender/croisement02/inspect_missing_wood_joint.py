"""Compare supplemental stems with approved neighbouring crowns, privately."""
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
from catalog import OUT, tree_workspace, scenery_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from stage_review_scene import signature
from tree_geometry import SIN, COS, RAY


def inspect(stem, neighbour, extra_neighbours=()):
    asset=f'croisement02-supplemental-wood-{stem:02}'
    candidate=scenery_workspace(asset)
    integrated=(candidate/'inspection/authored-integration.json').exists()
    if not integrated:candidate=OUT/f'missing-wood-round-1/assets/{asset}'
    suffix = ''.join(f'-{n:02}' for n in extra_neighbours) + ('-integrated' if integrated else '')
    destination = OUT / f'missing-wood-review/joint-{stem:02}-{neighbour:02}{suffix}'
    destination.mkdir(exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.preferences.filepaths.save_version = 0
    scene = bpy.context.scene
    scene.name = 'Supplemental stem joint inspection'
    collection = bpy.data.collections.new('Joint candidate Working')
    scene.collection.children.link(collection)
    records = []
    latest = {r['asset_id']: r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    inputs = [(tree_workspace(n), 'approved neighbour') for n in (neighbour, *extra_neighbours)]
    inputs.append((candidate, 'unapproved supplemental stem'))
    for worker, role in inputs:
        model = worker / 'model.blend'; original_hash = sha(model)
        if role == 'approved neighbour':
            decision = latest[worker.name]
            if decision['decision'] != 'approved' or decision['model_sha256'] != original_hash:
                raise ValueError('Neighbour is not the current approved geometry')
        audit = json.loads((worker / 'inspection/saved-model-audit.json').read_text())
        assert audit['status'] == 'PASS' and audit['model_sha256'] == original_hash
        with bpy.data.libraries.load(str(model), link=False) as (source, target):
            target.objects = [record['object'] for record in audit['objects']]
        evidence = []
        for obj in target.objects:
            collection.objects.link(obj)
        bpy.context.view_layer.update()
        for obj in target.objects:
            before = signature(obj); matrix = obj.matrix_world.copy()
            obj.parent = None; obj.matrix_world = matrix; obj.hide_render = False
            assert signature(obj) == before
            assert max(abs(obj.matrix_world[i][j] - matrix[i][j]) for i in range(4) for j in range(4)) < 1e-5
            evidence.append(dict(name=obj.name, source_node=obj['source_node'], surface_sha256=before,
                                 matrix_world=[list(row) for row in matrix], component=obj.get('projection_component')))
        records.append(dict(worker=str(worker), model_sha256=original_hash, role=role, objects=evidence))
    points = [obj.matrix_world @ v.co for obj in collection.objects for v in obj.data.vertices]
    center = Vector([(min(p[i] for p in points)+max(p[i] for p in points))/2 for i in range(3)])
    scene.render.engine = 'CYCLES'; scene.cycles.samples = 8; scene.cycles.transparent_max_bounces = 64
    scene.render.resolution_x = 640; scene.render.resolution_y = 640; scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = 'PNG'; scene.render.image_settings.color_mode = 'RGBA'
    scene.render.film_transparent = True
    scene.view_settings.view_transform = 'Standard'; scene.view_settings.look = 'None'
    world = bpy.data.worlds.new('Neutral'); world.use_nodes = True
    world.node_tree.nodes['Background'].inputs['Color'].default_value = (.4, .4, .4, 1)
    world.node_tree.nodes['Background'].inputs['Strength'].default_value = .7; scene.world = world
    data = bpy.data.cameras.new('Joint orthographic'); data.type = 'ORTHO'; data.clip_end = 20000
    camera = bpy.data.objects.new(data.name, data); scene.collection.objects.link(camera); scene.camera = camera
    views = []
    directions = [('source', RAY)] + [(f'oblique-{i}', Vector((math.sin(i*math.tau/4)*math.cos(math.radians(25)), -math.cos(i*math.tau/4)*math.cos(math.radians(25)), math.sin(math.radians(25))))) for i in range(4)]
    for label, direction in directions:
        camera.location = center + direction * 5000
        camera.rotation_euler = (center-camera.location).to_track_quat('-Z', 'Y').to_euler()
        bpy.context.view_layer.update()
        local = [camera.matrix_world.inverted() @ p for p in points]
        extent = max(max(p[i] for p in local)-min(p[i] for p in local) for i in (0, 1))
        data.ortho_scale = extent*1.15
        offset = Vector([(max(p[i] for p in local)+min(p[i] for p in local))/2 for i in (0, 1)] + [0])
        camera.location += camera.matrix_world.to_quaternion() @ offset
        for state in ('complete', 'wood-only'):
            for obj in collection.objects:
                obj.hide_render = state == 'wood-only' and obj.get('projection_component') == 'crown'
            path = destination / f'{label}-{state}.png'
            scene.render.filepath = str(path); bpy.ops.render.render(write_still=True)
            views.append(dict(image=path.name, sha256=sha(path), location=list(camera.location), rotation=list(camera.rotation_euler), ortho_scale=data.ortho_scale, state=state))
    for obj in collection.objects:
        obj.hide_render = False
    bpy.ops.wm.save_as_mainfile(filepath=str(destination / 'scene.blend'))
    sheet = Image.new('RGB', (1600, 680), '#333333'); draw = ImageDraw.Draw(sheet)
    for index, (label, _) in enumerate(directions):
        for row, state in enumerate(('complete', 'wood-only')):
            path = destination / f'{label}-{state}.png'; im = Image.open(path).convert('RGBA'); im.thumbnail((320, 320))
            sheet.paste(im, (index*320, row*340+20), im)
            draw.text((index*320+5,row*340+3), f'{label} / {state}', fill='white')
    sheet.save(destination / 'sheet.png')
    for record in records:
        assert sha(Path(record['worker'])/'model.blend') == record['model_sha256']
    source_view = next(v for v in views if v['image'] == 'source-complete.png')
    x, y, z = source_view['location']; half = source_view['ortho_scale']/2
    source_y = -y*SIN-z*COS
    source_box = [x-half, source_y-half, x+half, source_y+half]
    source = Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA')
    crop = source.transform((640, 640), Image.Transform.EXTENT, source_box, Image.Resampling.BICUBIC)
    crop.save(destination/'native-source.png')
    overlay = Image.alpha_composite(crop, Image.open(destination/'source-complete.png').convert('RGBA'))
    overlay.save(destination/'source-overlay.png')
    write_json(destination/'source-framing.json', dict(native_source_box=source_box,
        native_source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),
        images={name:sha(destination/name) for name in ('native-source.png', 'source-overlay.png')}))
    write_json(destination / 'evidence.json', dict(status='private joint comparison; visual review pending',
        model_sha256=sha(destination/'scene.blend'), inputs=records, views=views, sheet_sha256=sha(destination/'sheet.png'),
        caveat='No new crown, catalog, ownership assignment or approved geometry was changed. Wood-only rows suppress canopy for spatial diagnosis.'))


if __name__ == '__main__':
    acquire()
    try:
        if '--extended' in sys.argv:
            inspect(9, 8, (7,))
        else:
            inspect(9, 8)
            inspect(44, 45)
    finally:
        release()
