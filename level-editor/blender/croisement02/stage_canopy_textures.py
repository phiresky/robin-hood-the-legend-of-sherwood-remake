"""Stage only exactly approved canopy receivers, without publishing a map."""
import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT / 'level-editor/refinement'),
                str(ROOT / 'level-editor/refinement/blender')]
import bpy
from approved_texture_stage import select, inspect, geometry, appearance, require
from catalog import tree_workspace
from evidence_io import sha, write_json
from render_slots import acquire, release


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('decisions', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    output = args.output.resolve()
    require(not output.exists(), 'Staging destination must be fresh')
    decisions = args.decisions.resolve()
    document = json.loads(decisions.read_text())
    models = {r['asset_id']: tree_workspace(int(r['asset_id'].rsplit('-', 1)[1])) / 'model.blend'
              for r in document['decisions']}
    selected = select(decisions, models)
    require(len(selected) == 13, 'Expected exact thirteen canopy approvals')
    output.mkdir(parents=True)
    acquire()
    try:
        inspect(selected, models)
        # All import references are captured before constructing the private scene.
        bpy.ops.wm.read_factory_settings(use_empty=True)
        scene = bpy.context.scene
        scene.name = 'Croisement02 Approved Canopy Textures'
        collection = bpy.data.collections.new('Approved canopy receivers')
        scene.collection.children.link(collection)
        records = []
        for asset, record in selected.items():
            names = record['receiver_names']
            with bpy.data.libraries.load(record['model'], link=False) as (source, loaded):
                require(set(names) <= set(source.objects), 'Approved receivers missing')
                loaded.objects = list(names)
            objects = list(loaded.objects)
            for obj in objects:
                collection.objects.link(obj)
                ancestor = obj.parent
                while ancestor is not None:
                    require(ancestor.type != 'MESH', 'Mesh ancestor outside texture scope')
                    if ancestor.name not in scene.objects:
                        collection.objects.link(ancestor)
                    ancestor = ancestor.parent
            bpy.context.view_layer.update()
            refs = []
            for name, obj in zip(names, objects):
                reference = record['objects'][name]
                require(geometry(obj) == reference['geometry']
                        and appearance(obj) == reference['appearance'], 'Imported receiver changed: ' + name)
                refs.append(dict(original_name=name, imported_name=obj.name, **reference))
            records.append(dict(asset_id=asset, model=record['model'],
                approved_model_sha256=record['decision']['model_sha256'],
                geometry_model_sha256=sha(models[asset]),
                texture_review_revision=record['decision']['review_revision'], objects=refs))
        bpy.context.preferences.filepaths.save_version = 0
        model = output / 'scene.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model), compress=True)
        bpy.ops.wm.open_mainfile(filepath=str(model))
        expected = set()
        for row in records:
            for ref in row['objects']:
                obj = bpy.data.objects[ref['imported_name']]
                require(geometry(obj) == ref['geometry'] and appearance(obj) == ref['appearance'],
                        'Reopened approved receiver changed: ' + obj.name)
                expected.add(obj.name)
        require({o.name for o in bpy.context.scene.objects if o.type == 'MESH'} == expected,
                'Unexpected meshes in scoped canopy stage')
        select(decisions, models)
        write_json(output / 'selected-texture-approvals.json', selected)
        write_json(output / 'assembly.json', dict(status='PASS', scope='exact thirteen approved canopy assets only',
            publication='not performed', model_sha256=sha(model), decisions=str(decisions),
            decisions_sha256=sha(decisions), assets=records, meshes=len(expected),
            imported_geometry_and_appearance='PASS', reopened_preservation='PASS',
            original_approved_models_unchanged='PASS', full_map_integration='pending'))
        print('PASS: exact thirteen canopy textures staged and reopened', flush=True)
    finally:
        release()


if __name__ == '__main__':
    main()
