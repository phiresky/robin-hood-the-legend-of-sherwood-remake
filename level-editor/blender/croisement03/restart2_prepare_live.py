"""Prepare five approved assets against the current map without writing live files."""
import copy
import hashlib
import json
import math
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from unify_map_assets import stage
from hybrid_library import stage_hybrid
from asset_index import write_asset_index
from canonical_assets import read_model

WORK = ROOT / 'level-editor/work/croisement03-refinement/restart2'
LIVE = ROOT / 'level-editor/library'
OUT = WORK / 'publication-five-v1'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + '\n')


def main():
    OUT.mkdir(exist_ok=False)
    source = WORK / 'integration-round1/stage-v1/3d-assets'
    shutil.copytree(source, OUT / 'input/3d-assets')
    (OUT / 'input/scenes').mkdir()
    live_map = LIVE / 'scenes/croisement03.rhlos-map.json'
    document = json.loads(live_map.read_text())
    guards = {str(live_map): sha(live_map)}
    old_id = 'croisement03-group-049'
    old_ref = next(r for r in document['assetSources'] if r['id'] == old_id)
    old = json.loads((LIVE / old_ref['descriptor']).read_text())
    assert [p['node'] for p in old['parts']] == ['building-049']
    fire_path = OUT / 'input/3d-assets/croisement03-southwest-firewood-stack/asset.json'
    fire = json.loads(fire_path.read_text())
    # Existing gameplay has no coordinate-bearing records. Retain every field exactly.
    assert all(not old['gameplay'][k] for k in ('surfaces', 'movementBlockers', 'doors', 'lifts', 'interiors'))
    fire['gameplay'] = copy.deepcopy(old['gameplay'])
    write(fire_path, fire)
    write_asset_index(OUT / 'input/3d-assets')
    stage(OUT / 'input', OUT / 'canonical/library')
    stage_hybrid(OUT / 'canonical/library', OUT / 'map-assets')
    index = json.loads((OUT / 'map-assets/3d-assets/index.json').read_text())
    ids = [a['id'] for a in index['assets']]
    # Audit all actual referenced meshes, including compound groups.
    meshes = []
    for ref in document['sceneAssets'] + document['assetSources']:
        model, _, _ = read_model(LIVE / ref['model'], LIVE)
        meshes.extend(n['name'] for n in model['nodes'] if 'mesh' in n)
        descriptor = json.loads((LIVE / ref['descriptor']).read_text())
        paths = [ref['model'], ref['descriptor']] + [r['path'] for r in descriptor.get('resources', [])]
        for relative in paths:
            original, target = LIVE / relative, OUT / 'map-assets' / relative
            guards[str(original)] = sha(original)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(original, target)
    assert sorted(meshes) == sorted(['ground'] + [f'building-{i:03d}' for i in range(106)])
    before = copy.deepcopy(document)
    document['assetSources'] = [r for r in document['assetSources'] if r['id'] != old_id]
    document['placements'] = [p for p in document['placements'] if p['assets'] != [old_id]]
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    proof = {}
    for entry in index['assets']:
        path = OUT / 'map-assets/3d-assets' / entry['descriptor']
        desc = json.loads(path.read_text())
        origin = desc['source_origin_scene']
        transform = dict(dx=origin[0], dy=-origin[1] * sine, dz=origin[2] * cosine, rot_deg=0)
        document['assetSources'].append(dict(id=entry['id'], descriptor='3d-assets/' + entry['descriptor'],
            descriptor_sha256=sha(path), model='3d-assets/' + entry['model'],
            model_sha256=sha(OUT / 'map-assets/3d-assets' / entry['model']), model_scene=desc['model_scene']))
        document['placements'].append(dict(id=entry['id'], transform=transform, assets=[entry['id']]))
        if entry['id'] == fire['id']:
            previous = next(p for p in before['placements'] if p['assets'] == [old_id])['transform']
            old_points, new_points = old['parts'][0]['obstacle_local_game']['points'], desc['parts'][0]['obstacle_local_game']['points']
            errors = []
            for a, b in zip(old_points, new_points, strict=True):
                for key, offset in [('x', 'dx'), ('y', 'dy'), ('z_bottom', 'dz'), ('z_top', 'dz')]:
                    errors.append(abs(a[key] + previous[offset] - b[key] - transform[offset]))
            assert max(errors) < 1e-9, errors
            assert desc['gameplay'] == old['gameplay']
            assert {k:v for k,v in old['parts'][0]['obstacle_local_game'].items() if k != 'points'} == {
                k:v for k,v in desc['parts'][0]['obstacle_local_game'].items() if k != 'points'}
            proof = dict(world_obstacle_max_error=max(errors), gameplay_exact=True, obstacle_flags_exact=True)
    assert [p for p in document['placements'] if p['assets'][0] not in ids] == [p for p in before['placements'] if p['assets'] != [old_id]]
    write_asset_index(OUT / 'map-assets/3d-assets')
    write(OUT / 'croisement03.rhlos-map.json', document)
    write(OUT / 'map-assets/scenes/croisement03.rhlos-map.json', document)
    write(OUT / 'scope.json', dict(asset_ids=ids, already_published=[]))
    write(OUT / 'preparation.json', dict(status='PRIVATE_PREPARED', live_guards=guards,
        old_meshes=meshes, new_asset_ids=ids, firewood=proof, unchanged_other_placements=97,
        limitations=['Ground and surrounding native meshes remain unfinished; no full-map completion claim.']))
    print(OUT)


if __name__ == '__main__':
    main()
