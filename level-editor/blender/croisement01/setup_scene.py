"""Freeze Croisement01's reconstruction and source evidence for a full-map grouping pass."""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/croisement01-refinement'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    import bpy
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
    from setup_map import setup_map
    from refinement_inventory import inventory
    baseline = OUT / 'baseline'
    baseline.mkdir(parents=True, exist_ok=False)
    archive = ROOT / 'level-editor/work/scene-backups/map-manifest-migration/scenes'
    data = ROOT / 'datadirs/fullgame_gog_hackable/Data/Levels'
    inputs = {}
    for source, name in [
        (archive / 'croisement01-volumes.scene.glb', 'croisement01-volumes.scene.glb'),
        (archive / 'croisement01-volumes.scene.json', 'croisement01-volumes.scene.json'),
        (data / 'Croisement01.rhp.json', 'Croisement01.rhp.json'),
        (data / 'Day/Croisement01.map.png', 'revealed.png'),
        (OUT / 'source-states/covered.png', 'covered.png'),
        (ROOT / 'level-editor/library/scenes/croisement01.rhlos-map.json', 'published-map.json'),
    ]:
        shutil.copy2(source, baseline / name)
        inputs[name] = {'source': str(source), 'sha256': sha(source)}
        assert sha(baseline / name) == inputs[name]['sha256']
    shutil.copytree(data / 'Croisement01.rhp.d/masks', baseline / 'masks')
    inputs['masks'] = {str(p.relative_to(baseline / 'masks')): sha(p)
                       for p in sorted((baseline / 'masks').rglob('*')) if p.is_file()}
    setup_map(baseline / 'croisement01-volumes.scene.json', baseline / 'croisement01-baseline.blend')
    working = bpy.data.collections['Croisement01 Working']
    for obj in working.all_objects:
        source = obj['source_obstacle']
        obj['source_node'] = source.replace('terrace-', 'building-')
        if source.startswith('terrace-'):
            obj['source_kind'] = 'terrace'
            obj['source_obstacle'] = obj['source_node']
    bpy.context.preferences.filepaths.save_version = 0
    bpy.context.scene['source_artwork'] = str(baseline / 'covered.png')
    bpy.context.scene['frozen_baseline'] = True
    bpy.ops.wm.save_as_mainfile(filepath=str(baseline / 'croisement01-baseline.blend'))
    inventory(OUT / 'inventory', collection_name=working.name, map_name='Croisement01',
              source_path=baseline / 'covered.png', patch_manifest=OUT / 'source-states/layers.json')
    path = OUT / 'inventory/inventory.json'
    record = json.loads(path.read_text())
    for item in record['objects']:
        obj = bpy.data.objects[item['object']]
        item.update(parent=obj.parent.name if obj.parent else None,
                    matrix_world=[list(row) for row in obj.matrix_world],
                    geometry_sha256=hashlib.sha256(json.dumps({
                        'vertices': [list(v.co) for v in obj.data.vertices],
                        'faces': [list(p.vertices) for p in obj.data.polygons],
                    }, sort_keys=True).encode()).hexdigest(),
                    uv_sha256=hashlib.sha256(json.dumps({
                        layer.name: [list(loop.uv) for loop in layer.data]
                        for layer in obj.data.uv_layers
                    }, sort_keys=True).encode()).hexdigest(),
                    materials=[m.name if m else None for m in obj.data.materials])
    record['source_blend_sha256'] = sha(baseline / 'croisement01-baseline.blend')
    path.write_text(json.dumps(record, indent=2) + '\n')
    inputs['croisement01-baseline.blend'] = {'sha256': record['source_blend_sha256']}
    (baseline / 'manifest.json').write_text(json.dumps({'inputs': inputs, 'recipe_sha256': sha(__file__)}, indent=2) + '\n')
    print(json.dumps({'inventory': str(path), 'objects': len(record['objects'])}))


if __name__ == '__main__':
    main()
