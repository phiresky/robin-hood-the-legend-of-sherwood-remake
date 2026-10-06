"""Preserve both native tree references through the approved export pivot."""
import copy
import json
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from asset_index import write_asset_index
from review_evidence import sha

R = ROOT / 'level-editor/work/croisement01-refinement/restart2/tree19-integration-v2'
scene_path = ROOT / 'level-editor/library/scenes/croisement01.rhlos-map.json'
scene = json.loads(scene_path.read_text())
path = R / 'assets/croisement01-tree-19/asset.json'
descriptor = json.loads(path.read_text())
assert 'gameplay' not in descriptor
original = copy.deepcopy(descriptor)
x, y, z = descriptor['source_origin_scene']
pivot = [x, -y * math.sin(math.radians(35)), z * math.cos(math.radians(35))]
gameplay = None
records = []
for number in ['050', '051']:
    asset = 'croisement01-group-' + number
    source = ROOT / 'level-editor/library/3d-assets/croisement01' / asset / 'asset.json'
    old = json.loads(source.read_text())
    native = old['gameplay']
    expected = {'version', 'collision', 'surfaces', 'sightOrder', 'doors', 'lifts', 'interiors', 'draft'}
    assert set(native) == expected | ({'movementBlockers'} if number == '050' else set())
    assert native['version'] == 1 and native['collision'] == 'parts'
    assert all(native[key] == [] for key in ['surfaces', 'doors', 'lifts', 'interiors'])
    assert set(native['draft']) == {'issues'}
    placements = [p for p in scene['placements'] if p['assets'] == [asset]]
    assert len(placements) == 1
    transform = placements[0]['transform']
    assert transform['rot_deg'] == 0
    before = [transform['dx'], transform['dy'], transform['dz']]
    if gameplay is None:
        gameplay = copy.deepcopy(native)
        gameplay.update(sightOrder={}, movementBlockers=[], draft={'issues': []})
    assert not set(gameplay['sightOrder']) & set(native['sightOrder'])
    gameplay['sightOrder'].update(native['sightOrder'])
    for issue in native['draft']['issues']:
        if issue not in gameplay['draft']['issues']:
            gameplay['draft']['issues'].append(issue)
    errors = []
    for blocker in native.get('movementBlockers', []):
        assert blocker['holes'] == [] and len(blocker['polygon']) == len(blocker['height'])
        translated = copy.deepcopy(blocker)
        translated['polygon'] = [[px + before[0] - pivot[0], py + before[1] - pivot[1]] for px, py in blocker['polygon']]
        translated['height'] = [height + before[2] - pivot[2] for height in blocker['height']]
        for (px, py), height, (qx, qy), other in zip(blocker['polygon'], blocker['height'], translated['polygon'], translated['height']):
            errors += [abs(px + before[0] - qx - pivot[0]), abs(py + before[1] - qy - pivot[1]), abs(height + before[2] - other - pivot[2])]
        assert {k: v for k, v in blocker.items() if k not in ['polygon', 'height']} == {k: v for k, v in translated.items() if k not in ['polygon', 'height']}
        gameplay['movementBlockers'].append(translated)
    assert len(old['parts']) == 1
    part = old['parts'][0]
    replacement = next(row for row in descriptor['parts'] if row['node'] == part['node'])
    a, b = part['obstacle_local_game'], replacement['obstacle_local_game']
    assert {k: v for k, v in a.items() if k != 'points'} == {k: v for k, v in b.items() if k != 'points'}
    assert len(a['points']) == len(b['points'])
    for point, other in zip(a['points'], b['points']):
        assert set(point) == set(other)
        for key, index in [('x', 0), ('y', 1), ('z_bottom', 2), ('z_top', 2)]:
            errors.append(abs(point[key] + before[index] - other[key] - pivot[index]))
        assert {k: v for k, v in point.items() if k not in ['x', 'y', 'z_bottom', 'z_top']} == {k: v for k, v in other.items() if k not in ['x', 'y', 'z_bottom', 'z_top']}
    assert max(errors, default=0) < 1e-9
    records.append(dict(source_descriptor=str(source), source_descriptor_sha256=sha(source), original_gameplay=native, original_placement=placements[0], world_coordinate_max_error=max(errors, default=0)))
backup = R / 'original-export-asset.json'
assert not backup.exists()
backup.write_text(json.dumps(original, indent=2) + '\n')
descriptor['gameplay'] = gameplay
path.write_text(json.dumps(descriptor, indent=2) + '\n')
write_asset_index(R / 'assets')
(R / 'metadata-preservation.json').write_text(json.dumps(dict(status='PASS', scope='Preserve existing records and all draft issues; pivot translation only, no new parity claim', records=records, source_scene_sha256=sha(scene_path), original_descriptor_sha256=sha(backup), descriptor_sha256=sha(path), merged_gameplay=gameplay), indent=2) + '\n')
print('PASS: both references, movement contour, obstacle volumes and all draft issues preserved')
