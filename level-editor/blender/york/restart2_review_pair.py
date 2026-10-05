"""Combine independently projected pair materials and reopen actual reviews."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
HOUSE = 'york-southwest-square-west-house'
BAY = 'york-market-southeast-tall-narrow-house'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--revision', default='assembled-review')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    candidate = args.candidate.resolve()
    output = candidate / args.revision
    if output.exists():
        raise FileExistsError(output)
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire, release
    acquire()
    import bpy
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import inspect_geometry
    bay_workspace = candidate / 'assets' / BAY
    house_workspace = candidate / 'assets' / HOUSE
    bpy.ops.wm.open_mainfile(filepath=str(bay_workspace / 'model.blend'))
    scene = bpy.data.scenes['york Refinement']
    bpy.context.window.scene = scene
    output.mkdir()

    def shape(obj):
        return {'points': [tuple(obj.matrix_world @ v.co) for v in obj.data.vertices],
                'faces': [tuple(p.vertices) for p in obj.data.polygons]}

    names = json.loads((house_workspace / 'input/views.json').read_text())['object_names']
    targets = {name: bpy.data.objects[name] for name in names}
    before = {name: shape(obj) for name, obj in targets.items()}
    with bpy.data.libraries.load(str(house_workspace / 'model.blend'), link=False) as (available, imported):
        if not set(names) <= set(available.objects):
            raise ValueError('House projection donor lacks reviewed objects')
        imported.objects = list(names)
    copied = []
    for name, donor in zip(names, imported.objects):
        target = targets[name]
        if donor['source_node'] != target['source_node'] or donor['asset_group'] != HOUSE:
            raise ValueError('Projection donor ownership mismatch')
        target.data = donor.data.copy()
        copied.append(name)
    for obj in imported.objects:
        bpy.data.objects.remove(obj, do_unlink=True)
    after = {name: shape(obj) for name, obj in targets.items()}
    if before != after:
        (output/'geometry-failure.json').write_text(json.dumps({name:{
            'before_vertices':len(before[name]['points']), 'after_vertices':len(after[name]['points']),
            'before_faces':len(before[name]['faces']), 'after_faces':len(after[name]['faces']),
            'first_before':before[name]['points'][:3], 'first_after':after[name]['points'][:3]}
            for name in before if before[name]!=after[name]},indent=2)+'\n')
        raise ValueError('Material assembly changed geometry')
    bpy.context.preferences.filepaths.save_version = 0
    combined = output / 'pair-materials.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(combined), compress=True)
    combined_sha = hashlib.sha256(combined.read_bytes()).hexdigest()
    display_group = 'york-paired-house-inspection'
    for obj in scene.objects:
        if obj.type == 'MESH' and obj.get('asset_group') in (HOUSE, BAY):
            obj['asset_group'] = display_group
    display = output / 'pair-display.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(display), compress=True)
    record = {'status': 'private inspection; no approval or publication',
              'pair_model_sha256': combined_sha, 'geometry_preserved_during_material_assembly': True,
              'projected_house_objects': copied,
              'source_workers': {asset: hashlib.sha256((candidate/'assets'/asset/'model.blend').read_bytes()).hexdigest()
                                 for asset in (BAY,HOUSE)},
              'paired_display_scope': 'Only asset_group display selectors are temporarily joined; geometry, UV, materials and native source identities remain unchanged.'}
    (output / 'assembly.json').write_text(json.dumps(record, indent=2)+'\n')
    for label, asset, source, crop in (
            ('bay', BAY, combined, [545,1200,680,1430]),
            ('house', HOUSE, combined, [550,1100,790,1460]),
            ('pair', display_group, display, [550,1100,790,1460])):
        review = output / label
        review.mkdir()
        os.symlink(source, review / 'model.blend')
        config = json.loads((bay_workspace / 'workspace.json').read_text())
        config['asset_id'] = asset
        (review / 'workspace.json').write_text(json.dumps(config, indent=2)+'\n')
        sys.argv = [str(inspect_geometry.__file__), '--', str(review), '--crop', *map(str,crop)]
        inspect_geometry.main()
        release()
    print(json.dumps(record))


if __name__ == '__main__':
    main()
