"""Run with Blender --background --factory-startup --python-exit-code 1 --python this_file."""
import json
from pathlib import Path
import sys
import tempfile

import bpy
from mathutils import Matrix

sys.path.insert(0, str(Path(__file__).resolve().parent))
from group_assets import group_assets, reconcile_asset_groups, sync_asset_names
from test_catalog_schema import catalog


def run():
    bpy.context.scene.name = 'Example Refinement'
    working = bpy.data.collections.new('Example Working')
    bpy.context.scene.collection.children.link(working)
    root = bpy.data.objects.new('Root', None)
    root['source_obstacle'] = 'map'
    working.objects.link(root)
    objects = []
    for number, component in enumerate(('west', 'east', None)):
        mesh = bpy.data.meshes.new('mesh')
        mesh.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0)], [], [(0, 1, 2)])
        obj = bpy.data.objects.new(str(component), mesh)
        working.objects.link(obj)
        obj.parent = root
        obj['source_obstacle'] = 'building-200'
        if component:
            obj['projection_component'] = component
        obj.hide_render = component is None
        obj.hide_viewport = component is None
        obj.matrix_world = Matrix.Translation((number * 4, 2, 8))
        objects.append(obj)
    bpy.context.view_layer.update()
    matrices = [obj.matrix_world.copy() for obj in objects]
    with tempfile.TemporaryDirectory(prefix='component-catalog-') as directory:
        path = Path(directory) / 'catalog.json'
        path.write_text(json.dumps(catalog(1)))
        bpy.ops.wm.save_as_mainfile(filepath=str(Path(directory) / 'test.blend'))
        group_assets(path)
        assert all(obj['asset_group'] == 'house' for obj in objects)
        # Nonidentity group pivots exercise preservation when moving east.
        objects[0].parent.location.x = 11
        bpy.context.view_layer.update()
        matrices = [obj.matrix_world.copy() for obj in objects]
        value = catalog()
        path.write_text(json.dumps(value))
        report = reconcile_asset_groups(path)
        assert report['canonical_parts'] == 1
        assert [obj['asset_group'] for obj in objects] == ['house', 'annex', 'house']
        assert report['max_transform_drift'] < 1e-5
        assert objects[-1].hide_render and objects[-1].hide_viewport
        for obj, before in zip(objects, matrices):
            assert all(abs(obj.matrix_world[r][c] - before[r][c]) < 1e-5 for r in range(4) for c in range(4))
        value['groups'][1]['name'] = 'East annex'
        path.write_text(json.dumps(value))
        sync_asset_names(path)
        assert objects[1]['asset_name'] == 'East annex'
        assert objects[1].name.startswith('East annex / Wall')
        # An unrelated publication must preserve detached state meshes exactly.
        detached = objects[1]
        world = detached.matrix_world.copy(); detached.parent = None; detached.matrix_world = world
        detached.name = 'Retained revealed state'
        bpy.context.view_layer.update()
        before = (detached.name, detached.parent, [list(row) for row in detached.matrix_world])
        reconcile_asset_groups(path, preserve_objects=[detached])
        assert before == (detached.name, detached.parent, [list(row) for row in detached.matrix_world])
        detached['asset_group'] = 'house'
        try:
            reconcile_asset_groups(path, preserve_objects=[detached])
        except ValueError as error:
            assert 'ownership' in str(error)
        else:
            raise AssertionError('Preserved object silently changed ownership')
        detached['asset_group'] = 'annex'
        # Validation must fail before relabeling or reparenting anything.
        value['groups'][0]['parts'][0]['components'].append('missing')
        path.write_text(json.dumps(value))
        before = [(obj.name, obj.parent) for obj in objects]
        for operation in (reconcile_asset_groups, sync_asset_names):
            try:
                operation(path)
            except ValueError as error:
                assert 'Missing component' in str(error)
            else:
                raise AssertionError('Missing component was accepted')
            assert before == [(obj.name, obj.parent) for obj in objects]
    print('Component grouping integration PASS')


if __name__ == '__main__':
    run()
