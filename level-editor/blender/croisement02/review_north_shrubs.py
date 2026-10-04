"""Render pending shrubs65/66 beside the audited bank and approved trees12–14."""
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
from catalog import OUT, scenery_workspace, tree_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, RAY


def main():
    bank = scenery_workspace('croisement02-north-woodland-bank')
    trees = [tree_workspace(i) for i in (12, 13, 14)]
    shrubs = [OUT / f'understory-round-1/assets/croisement02-shrub-{i}' for i in (65, 66)]
    refit=OUT/'understory-round-2/assets/croisement02-shrub-66'
    if (refit/'inspection/refit-evidence.json').exists():
        receipt=json.loads((refit/'inspection/refit-evidence.json').read_text())
        if receipt['model_sha256']!=sha(refit/'model.blend'):raise ValueError('Refitted shrub66 model changed')
        shrubs[1]=refit
    workers = [bank, *trees, *shrubs]
    hashes = [sha(w / 'model.blend') for w in workers]
    decisions = {r['asset_id']: r for r in json.loads((OUT/'user-feedback.json').read_text())['records']}
    for worker in trees:
        decision = decisions[worker.name]
        if decision['decision'] != 'approved' or decision['model_sha256'] != sha(worker/'model.blend'):
            raise ValueError('Joint context tree is no longer the approved model: '+worker.name)
    for worker, model_hash in zip(workers, hashes):
        audit = json.loads((worker/'inspection/saved-model-audit.json').read_text())
        if audit['status'] != 'PASS' or audit['model_sha256'] != model_hash:
            raise ValueError('Current saved-model audit required: '+worker.name)
    destination = OUT/'north-shrub-joint-review'/('-'.join(h[:8] for h in (hashes[0], hashes[-2], hashes[-1])))
    destination.mkdir(parents=True, exist_ok=False)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    mesh_records = []
    meshes = []
    per_asset = {}
    for worker in workers:
        with bpy.data.libraries.load(str(worker/'model.blend'), link=False) as (source, loaded):
            if 'Croisement02 Working' not in source.collections:
                raise ValueError('Missing isolated working collection')
            loaded.collections = ['Croisement02 Working']
        collection = loaded.collections[0]
        scene.collection.children.link(collection)
        bpy.context.view_layer.update()
        selected = [o for o in collection.all_objects if o.type == 'MESH' and o.get('asset_group') == worker.name]
        if not selected:
            raise ValueError('No selected meshes: '+worker.name)
        if worker == bank and sorted(o.get('source_node') for o in selected) != [f'building-{i:03}' for i in range(5)]:
            raise ValueError('Bank scope differs from audited parts0–4')
        transforms = {o:o.matrix_world.copy() for o in selected}
        scene.collection.children.unlink(collection)
        per_asset[worker.name] = []
        for original in selected:
            obj = original.copy()
            obj.parent = None
            obj.matrix_world = transforms[original]
            obj.hide_render = False
            scene.collection.objects.link(obj)
            meshes.append(obj)
            per_asset[worker.name].append(obj)
            mesh_records.append(dict(name=obj.name, asset_group=worker.name, source_node=obj.get('source_node'),
                matrix=[list(row) for row in obj.matrix_world], transform_drift=0))
    scene.world = bpy.data.worlds.new('Neutral joint review world')
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
    light_data = bpy.data.lights.new('Joint sun', 'SUN')
    light_data.energy = 2
    light = bpy.data.objects.new(light_data.name, light_data)
    scene.collection.objects.link(light)
    light.rotation_euler = (math.radians(28), math.radians(-25), math.radians(-30))
    camera_data = bpy.data.cameras.new('Joint camera')
    camera_data.type = 'ORTHO'
    camera_data.sensor_fit = 'HORIZONTAL'
    camera_data.clip_end = 20000
    camera = bpy.data.objects.new(camera_data.name, camera_data)
    scene.collection.objects.link(camera)
    scene.camera = camera
    cameras = []

    def render(name, target, direction, scale, width, height):
        camera.location = target + direction*5000
        camera.rotation_euler = (target-camera.location).to_track_quat('-Z', 'Y').to_euler()
        camera_data.ortho_scale = scale
        bpy.context.view_layer.update()
        scene.render.resolution_x = width
        scene.render.resolution_y = height
        scene.render.filepath = str(destination/name)
        bpy.ops.render.render(write_still=True)
        cameras.append(dict(image=name, matrix=[list(row) for row in camera.matrix_world], ortho_scale=scale, hidden_for_contact=[o.name for o in meshes if o.hide_render]))

    crop = (1010, 35, 1360, 320)
    left, top, right, bottom = crop
    render('source-view.png', Vector(((left+right)/2, -(top+bottom)/2/SIN, 0)), RAY, right-left, right-left, bottom-top)
    source = Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop(crop)
    actual = Image.open(destination/'source-view.png').convert('RGBA')
    board = Image.new('RGB', (source.width*2, source.height+24), '#454545')
    board.paste(source, (0, 24)); board.paste(actual, (source.width, 24), actual)
    draw = ImageDraw.Draw(board)
    draw.text((4,4), 'Native source', fill='white')
    draw.text((source.width+4,4), 'Bank + trees12/13/14 + shrubs65/66', fill='white')
    board.resize((board.width*2, board.height*2), Image.Resampling.NEAREST).save(destination/'source-comparison.png')
    bounds = {}
    for worker in shrubs:
        points = [o.matrix_world@v.co for o in per_asset[worker.name] for v in o.data.vertices]
        lo = Vector([min(p[i] for p in points) for i in range(3)])
        hi = Vector([max(p[i] for p in points) for i in range(3)])
        center = (lo+hi)/2
        scale = max((hi-lo).length*1.25, 220)
        bounds[worker.name] = dict(minimum=list(lo), maximum=list(hi))
        sheet = Image.new('RGB', (2048,768), '#454545')
        for i in range(8):
            angle = math.pi*2*i/8
            direction = Vector((math.sin(angle)*math.cos(.55), -math.cos(angle)*math.cos(.55), math.sin(.55)))
            name = f'{worker.name}-view-{i}.png'
            render(name, center, direction, scale, 512,384)
            image = Image.open(destination/name).convert('RGBA')
            sheet.paste(image, ((i%4)*512, (i//4)*384), image)
        sheet.save(destination/f'{worker.name}-sheet.png')
        crowns=[o for o in meshes if o.get('projection_component')=='crown' and o.get('asset_group','').startswith('croisement02-tree-')]
        for obj in crowns:obj.hide_render=True
        contact=Image.new('RGB',(2048,408),'#454545')
        ImageDraw.Draw(contact).text((4,4),'Low-angle bank contact diagnostic; neighbouring tree crowns hidden, approved wood unchanged',fill='white')
        for i in range(4):
            angle=math.pi*2*i/4
            direction=Vector((math.sin(angle)*math.cos(.25),-math.cos(angle)*math.cos(.25),math.sin(.25)))
            name=f'{worker.name}-contact-{i}.png'
            render(name,center,direction,scale,512,384)
            image=Image.open(destination/name).convert('RGBA')
            contact.paste(image,(i*512,24),image)
        contact.save(destination/f'{worker.name}-contact-sheet.png')
        for obj in crowns:obj.hide_render=False
    combined = Image.new('RGB', (2048,1536))
    for i, worker in enumerate(shrubs):
        combined.paste(Image.open(destination/f'{worker.name}-sheet.png'), (0,i*768))
    combined.save(destination/'sheet.png')
    for worker, expected in zip(workers, hashes):
        if sha(worker/'model.blend') != expected:
            raise ValueError('Joint input changed during rendering')
    write_json(destination/'evidence.json', dict(workers=[dict(path=str(w),model_sha256=h) for w,h in zip(workers,hashes)],
        meshes=mesh_records, shrub_bounds=bounds, cameras=cameras, source_crop=list(crop),
        sheet_sha256=sha(destination/'sheet.png'), source_comparison_sha256=sha(destination/'source-comparison.png'),
        status='Candidate joint packet, manual visual review required. Terrain and other native plants absent; no approval inferred.'))
    print(destination, flush=True)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
