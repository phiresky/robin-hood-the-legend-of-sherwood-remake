"""Blender integration: source preservation, donor ownership, opaque RGBA export.

Run with blender --background --factory-startup --python this_file.py.
Requires NumPy, Pillow and ~/.cargo/bin/texture-synthesis.
"""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
import json
import struct
import tempfile
import bpy
import numpy as np
from mathutils import Vector
import source_projection_bake
import synthesize_owned_atlases

work = Path(__file__).parent.parent/'work'
work.mkdir(parents=True,exist_ok=True)
output = Path(tempfile.mkdtemp(prefix='owned-source-test-', dir=work))
collection = bpy.data.collections.new('Test Working')
bpy.context.scene.collection.children.link(collection)
image = bpy.data.images.new('source', width=64,height=64)
pixels = np.ones((64,64,4),dtype=np.float32)
pixels[:,:,:3] = (.6,.3,.15)
image.pixels.foreach_set(pixels.ravel())
image.filepath_raw = str(output/'source.png')
image.file_format = 'PNG'
image.save()
toward = Vector((0,-.819152044,.573576436))
points = [Vector((8,-40,0)),Vector((40,-40,0)),Vector((40,-8,0)),Vector((8,-8,0))]
for name,offset in [('front',Vector()),('occluded',-toward*10)]:
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([p+offset for p in points],[],[(0,1,2,3)])
    obj = bpy.data.objects.new(name,mesh)
    collection.objects.link(obj)
    obj['source_node'] = name
    obj['asset_group'] = 'test-building'

def atlas(obj):
    mat = obj.data.materials[0]
    image = next(n.image for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE')
    p = np.empty(image.size[0]*image.size[1]*4,dtype=np.float32)
    image.pixels.foreach_get(p)
    return p.reshape(image.size[1],image.size[0],4)

source_projection_bake.bake('Test',output/'source.png',output/'neutral.json')
before = {obj.name:atlas(obj).copy() for obj in collection.objects}
report = source_projection_bake.bake('Test',output/'source.png',output/'synthesized.json',hidden_fill='synthesized')
assert report['synthesis']['tiles'] >= 1, report
for obj in collection.objects:
    after = atlas(obj)
    known = after[:,:,3] == 1
    np.testing.assert_array_equal(after[known,:3],before[obj.name][known,:3])
    assert np.any(after[after[:,:,3]==0,:3] > .1)
    if obj.name == 'occluded':
        assert not known.any()
rebuilt = synthesize_owned_atlases.synthesize('Test',output/'postprocess')
assert rebuilt['synthesis']['tiles'] >= 1, rebuilt
for obj in collection.objects:
    after = atlas(obj)
    known = after[:,:,3] == 1
    np.testing.assert_array_equal(after[known,:3],before[obj.name][known,:3])
for obj in bpy.context.selected_objects:
    obj.select_set(False)
for obj in collection.objects:
    obj.select_set(True)
bpy.ops.export_scene.gltf(filepath=str(output/'test.glb'),export_format='GLB',use_selection=True,export_extras=True)
raw = (output/'test.glb').read_bytes()
size = struct.unpack_from('<I',raw,12)[0]
gltf = json.loads(raw[20:20+size])
for material in gltf['materials']:
    assert material.get('alphaMode','OPAQUE') == 'OPAQUE'
    assert material['extras']['source_ownership_fill'] == 'synthesized'
    assert 'KHR_materials_unlit' in material['extensions']
    assert 'baseColorTexture' in material['pbrMetallicRoughness']
print('PASS: strict ownership, unchanged observed RGB, actual CLI, opaque unlit RGBA export',output)
