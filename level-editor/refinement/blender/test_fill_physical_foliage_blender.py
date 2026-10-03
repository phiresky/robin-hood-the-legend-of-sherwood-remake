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
print('PASS: unknown RGB filled; source material, foreign shared image and physical alpha preserved')
