"""Blender fixture for geometry-guarded texture handoff imports."""
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bpy
from integrate_refinement import import_asset_textures


def check():
    collection = bpy.data.collections.new('Texture import fixture')
    bpy.context.scene.collection.children.link(collection)
    parent = bpy.data.objects.new('Named fixture asset', None)
    collection.objects.link(parent)
    parent.location = (5, 7, 9)
    mesh = bpy.data.meshes.new('Fixture triangle')
    mesh.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0)], [], [(0, 1, 2)])
    obj = bpy.data.objects.new('Fixture visible part', mesh)
    collection.objects.link(obj)
    obj.parent = parent
    obj['asset_group'] = 'fixture'
    obj['source_node'] = 'part-1'
    material = bpy.data.materials.new('Approved texture material')
    material['test_approved_texture'] = True
    mesh.materials.append(material)
    uv = mesh.uv_layers.new(name='Reviewed UV')
    for loop in uv.data:
        loop.uv = (.2, .7)
    bpy.context.view_layer.update()
    with tempfile.TemporaryDirectory() as directory:
        good = Path(directory) / 'good.blend'
        bad = Path(directory) / 'bad.blend'
        bpy.data.libraries.write(str(good), {obj})
        obj.data.vertices[0].co.z = 3
        bpy.data.libraries.write(str(bad), {obj})
        obj.data.vertices[0].co.z = 0
        obj.data.materials.clear()
        objects_before = set(bpy.data.objects)
        report = import_asset_textures(good, asset_id='fixture', collection_name=collection.name)
        assert report['geometry_unchanged'] and report['components'] == 1
        assert obj.data.materials[0]['test_approved_texture']
        assert abs(obj.data.uv_layers['Reviewed UV'].data[0].uv.x - .2) < 1e-6
        assert set(bpy.data.objects) == objects_before
        data_before = obj.data
        try:
            import_asset_textures(bad, asset_id='fixture', collection_name=collection.name)
        except ValueError as error:
            assert 'geometry or ownership' in str(error)
        else:
            raise AssertionError('Changed geometry handoff accepted')
        assert obj.data == data_before
        assert set(bpy.data.objects) == objects_before
    print('PASS: UV/material import preserves geometry and parenting; changed geometry rejected without partial import')


if __name__ == '__main__':
    check()
