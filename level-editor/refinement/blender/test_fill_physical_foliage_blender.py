"""Run with Blender --background --python-exit-code 1 --python this-file."""
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    import bpy
except ImportError:
    import unittest
    raise unittest.SkipTest('Requires Blender')
import numpy as np
from fill_physical_foliage import fill

mesh = bpy.data.meshes.new('test foliage')
mesh.from_pydata([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], [], [(0, 1, 2, 3)])
obj = bpy.data.objects.new('test canopy', mesh)
bpy.context.scene.collection.objects.link(obj)
uv = mesh.uv_layers.new(name='Foliage UV')
for i, point in enumerate([(0, 0), (1, 0), (1, 1), (0, 1)]):
    uv.data[i].uv = point
ownership = mesh.color_attributes.new(name='Source ownership', type='FLOAT_COLOR', domain='CORNER')
for entry in ownership.data:
    entry.color = (0, 1, 1, 1)
mat = bpy.data.materials.new('unknown foliage')
mat.use_nodes = True
mat['foliage_physical_opacity'] = True
mat['source_ownership_channel'] = 'vertex-color-r'
image = bpy.data.images.new('original cutout', width=4, height=4, alpha=True)
before = np.full((4, 4, 4), .4, dtype=np.float32)
before[..., 3] = 1
before[0, :, 3] = 0
image.pixels.foreach_set(before.ravel())
before = np.asarray(image.pixels[:], dtype=np.float32).reshape(4, 4, 4)
texture = mat.node_tree.nodes.new('ShaderNodeTexImage')
texture.image = image
mapping = mat.node_tree.nodes.new('ShaderNodeUVMap')
mapping.uv_map = uv.name
mat.node_tree.links.new(mapping.outputs['UV'], texture.inputs['Vector'])
mesh.materials.append(mat)
foreign_mesh = mesh.copy()
foreign = bpy.data.objects.new('foreign canopy sharing material', foreign_mesh)
bpy.context.scene.collection.objects.link(foreign)


def sample(obj, normal, positions, accepted, colors, *, face_index):
    colors[:, :3] = [.1, .7, .2]
    return np.ones(len(colors), bool)


reports = fill([obj], sample, None, 'test-generated-hash')
assert reports[0]['generated'] == 12, reports
replacement = obj.data.materials[0]
assert replacement != mat and foreign.data.materials[0] == mat
new_image = replacement.node_tree.nodes[texture.name].image
actual = np.asarray(new_image.pixels[:], dtype=np.float32).reshape(4, 4, 4)
assert np.array_equal(actual[..., 3], before[..., 3])
assert np.array_equal(actual[0], before[0])
assert np.array_equal(np.asarray(image.pixels[:]).reshape(4, 4, 4), before)
assert np.max(np.abs(actual[1:, :, :3] - [.1, .7, .2])) <= 1 / 255 + 1e-7
for entry in ownership.data:
    entry.color = (1, 1, 1, 1)
assert fill([obj], sample, None, 'unused') == []
# A pixel centre can be hidden while a quarter-pixel sample is visible.
# Sampling must remain within that same unknown texel and preserve its alpha.
for entry in ownership.data:
    entry.color = (0, 1, 1, 1)
obj.data.materials[0] = mat

def edge_sample(obj, normal, positions, accepted, colors, *, face_index, record_statistics=True):
    visible = np.mod(positions[:, 0] * 4, 1) > .6
    colors[visible, :3] = [.1, .7, .2]
    return visible

retried = fill([obj], edge_sample, None, 'test-subpixels', subpixels=True)
assert retried[0]['generated'] == 12 and retried[0]['subpixel_generated'] == 12, retried
image_retry = next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type == 'TEX_IMAGE')
actual = np.asarray(image_retry.pixels[:], dtype=np.float32).reshape(4, 4, 4)
assert np.array_equal(actual[..., 3], before[..., 3])
assert np.array_equal(actual[0], before[0])

# Dense sampling reaches slivers beyond the quarter-pixel offsets.
obj.data.materials[0] = mat

def sliver_sample(obj, normal, positions, accepted, colors, *, face_index, record_statistics=True):
    visible = np.mod(positions[:, 0] * 4, 1) > .8
    colors[visible, :3] = [.1, .7, .2]
    return visible

dense = fill([obj], sliver_sample, None, 'test-grid', subpixels=True, sample_grid=4)
assert dense[0]['generated'] == 12 and dense[0]['grid_generated'] == 12, dense
assert dense[0]['subpixel_generated'] == 0, dense
image_retry = next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type == 'TEX_IMAGE')
actual = np.asarray(image_retry.pixels[:], dtype=np.float32).reshape(4, 4, 4)
assert np.array_equal(actual[..., 3], before[..., 3])
assert np.array_equal(actual[0], before[0])
assert np.array_equal(np.asarray(image.pixels[:]).reshape(4, 4, 4), before)

# Repair a single occluded texel from generated neighbours, retaining alpha.
obj.data.materials[0] = mat

def hole_sample(obj, normal, positions, accepted, colors, *, face_index):
    visible = ~((positions[:, 0] > .8) & (positions[:, 1] > .8))
    colors[visible, :3] = [.1, .7, .2]
    return visible

edges = fill([obj], hole_sample, None, 'test-edges', edge_fill_radius={mat.name: 1})
assert edges[0]['generated'] == 11 and edges[0]['extrapolated'] == 1, edges
image_retry = next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type == 'TEX_IMAGE')
actual = np.asarray(image_retry.pixels[:], dtype=np.float32).reshape(4, 4, 4)
assert np.array_equal(actual[..., 3], before[..., 3])
assert np.array_equal(actual[0], before[0])
assert np.array_equal(np.asarray(image.pixels[:]).reshape(4, 4, 4), before)

try:
    fill([obj], hole_sample, None, 'invalid', edge_fill_radius={'absent material': 1})
    raise AssertionError('Foreign material policy accepted')
except ValueError as error:
    assert 'absent' in str(error)

import tempfile
with tempfile.TemporaryDirectory() as folder:
    path = str(Path(folder) / 'foliage.blend')
    bpy.ops.wm.save_as_mainfile(filepath=path)
    bpy.ops.wm.open_mainfile(filepath=path, load_ui=False)
    saved = bpy.data.objects['test canopy'].data.materials[0]
    saved_image = next(n.image for n in saved.node_tree.nodes if n.type == 'TEX_IMAGE')
    stored = np.asarray(saved_image.pixels[:], dtype=np.float32).reshape(4, 4, 4)
    assert np.array_equal(stored, actual)
print('PASS: unknown RGB filled; source material, foreign shared image and physical alpha preserved after saving')
