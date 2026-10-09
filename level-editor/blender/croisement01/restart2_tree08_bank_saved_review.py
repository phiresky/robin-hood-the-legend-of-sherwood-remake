"""Resume interrupted bank-model inspection from the immutable saved model."""
import gc
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
sys.path.insert(0, str(Path(__file__).parent))
from render_slots import acquire, release
from restart2_tree08_union import topology

R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
B = R / 'tree08-wood-prototype-v17-bank-bands'
O = B / 'review-resume-v1'
P = R / 'tree08-wood-prototype-v14-root-ray/model.blend'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert not O.exists()


def guard():
    assert shutil.disk_usage(R).free >= 10 * 1024**3 + 16 * 1024**2
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')) >= 6 * 1024**2


guard()
acquire()
O.mkdir()
digest = sha(B / 'model.blend')
parent_hash = sha(P)
assert parent_hash == '39c329562cdc4e232061ca4f583126a6747de90054f1bd11bba9c799cf0714cf'
s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
ray, down, right = Vector((0, -c, s)), Vector((0, -s, -c)), Vector((1, 0, 0))
bpy.ops.wm.open_mainfile(filepath=str(P))
obj = next(o for o in bpy.context.scene.objects if o.type == 'MESH')
mesh = obj.data
world = [obj.matrix_world @ v.co for v in mesh.vertices]
tree = BVHTree.FromPolygons(world, [list(f.vertices) for f in mesh.polygons])
core = np.asarray(Image.open(R / 'tree08-semantic-source-v1/bark-core-proposal.png')) > 0
selected = set()
for y, x in np.argwhere(core):
    hit = tree.ray_cast(right * (x + 331.5) + down * (y + 11.5) + ray * 2000, -ray, 4000)
    assert hit[2] is not None
    selected.add(hit[2])
expected = {tuple(tuple(world[v]) for v in mesh.polygons[i].vertices): [tuple(mesh.uv_layers.active.data[j].uv) for j in mesh.polygons[i].loop_indices] for i in selected}
image_hashes = {im.name: hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file}
del world, tree, mesh, obj
gc.collect()
bpy.ops.wm.open_mainfile(filepath=str(B / 'model.blend'))
scene = bpy.context.scene
obj = next(o for o in scene.objects if o.type == 'MESH')
world = [obj.matrix_world @ v.co for v in obj.data.vertices]
packet = np.load(R / 'tree08-bank-hug-affine-cpu-v4/candidate.npz')
assert np.max(abs(np.array(world) - packet['vertices'])) < .0001
assert np.array_equal(np.array([f.vertices[:] for f in obj.data.polygons]), packet['faces'])
matched = set()
for face in obj.data.polygons:
    key = tuple(tuple(world[v]) for v in face.vertices)
    if key in expected:
        assert [tuple(obj.data.uv_layers.active.data[j].uv) for j in face.loop_indices] == expected[key]
        matched.add(key)
assert matched == set(expected)
assert image_hashes == {im.name: hashlib.sha256(im.packed_file.data).hexdigest() for im in bpy.data.images if im.packed_file}
check = topology(obj)
assert not any(check[k] for k in ['zero_area_triangles', 'nonmanifold_edges', 'inconsistent_edge_winding'])
guard()
scene.render.threads_mode = 'FIXED'
scene.render.threads = 2
scene.render.resolution_x = scene.render.resolution_y = 384
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = 'PNG'
scene.render.film_transparent = False
scene.world = bpy.data.worlds.new('Bank saved-model review')
scene.world.color = (.1, .1, .1)
camera_data = bpy.data.cameras.new('Bank saved-model camera')
camera = bpy.data.objects.new(camera_data.name, camera_data)
scene.collection.objects.link(camera)
scene.camera = camera
camera_data.type = 'ORTHO'
camera_data.clip_end = 10000
target = Vector(tuple((min(v[k] for v in world) + max(v[k] for v in world)) / 2 for k in range(3)))
directions = [Vector((c * math.sin(i * math.tau / 8), -c * math.cos(i * math.tau / 8), s)) for i in range(8)]
extent = max(max(abs((v - target).dot(d.cross(Vector((0, 0, 1))).normalized())), abs((v - target).dot(d.cross(Vector((0, 0, 1))).normalized().cross(d)))) for v in world for d in directions)
camera_data.ortho_scale = extent * 2 * 1.12
scene.render.engine = 'BLENDER_WORKBENCH'
scene.display.shading.light = 'STUDIO'
scene.display.shading.color_type = 'SINGLE'
scene.display.shading.single_color = (.5, .5, .5)
scene.display.shading.show_shadows = True
scene.display.shading.show_cavity = True
sheet = Image.new('RGB', (1536, 816), '#222222')
draw = ImageDraw.Draw(sheet)
for i, direction in enumerate(directions):
    guard()
    camera.location = target + direction * 1800
    camera.rotation_euler = (-direction).to_track_quat('-Z', 'Y').to_euler()
    scene.render.filepath = str(O / f'solid-{i}.png')
    bpy.ops.render.render(write_still=True)
    sheet.paste(Image.open(O / f'solid-{i}.png').convert('RGB'), (i % 4 * 384, i // 4 * 408 + 24))
    draw.text((i % 4 * 384 + 5, i // 4 * 408 + 5), f'solid {i}' + (' native angle' if i == 0 else ''), fill='white')
sheet.save(O / 'solid.png')
scene.render.engine = 'CYCLES'
scene.cycles.samples = 4
scene.cycles.use_denoising = False
scene.cycles.device = 'CPU'
scene.render.resolution_x, scene.render.resolution_y = 446, 461
camera_data.ortho_scale = 461
native_target = right * 554 + down * 241.5
camera.location = native_target + ray * 1500
camera.rotation_euler = (-ray).to_track_quat('-Z', 'Y').to_euler()
scene.render.filepath = str(O / 'native.png')
guard()
bpy.ops.render.render(write_still=True)
assert sha(B / 'model.blend') == digest and sha(P) == parent_hash
receipt = dict(status='SAVED_PRESERVATION_PASS_VISUAL_JUDGMENT_PENDING', model_sha256=digest, parent_sha256=parent_hash,
               source_core_pixels=int(core.sum()), retained_source_faces=len(matched), source_core_geometry_uv_exact=True,
               all_packed_images_exact=True, topology=check, solid8_sha256=sha(O / 'solid.png'), native_sha256=sha(O / 'native.png'))
(O / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
assert sum(p.stat().st_size for p in O.iterdir()) <= 16 * 1024**2
release()
print(json.dumps(receipt), flush=True)
