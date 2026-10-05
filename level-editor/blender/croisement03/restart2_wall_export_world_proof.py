"""Check each private wall obstacle against its existing world placement."""
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
RUN = ROOT / 'level-editor/work/croisement03-refinement/restart2/wall-tree-integration-preflight'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    mapping = json.loads((RUN / 'mapping.json').read_text())
    live = Path(mapping['live_map'])
    assert sha(live) == mapping['live_map_sha256']
    export = RUN / 'wall-export-v1'
    receipt = json.loads((export / 'receipt.json').read_text())
    for name, digest in receipt['files'].items():
        assert sha(export / name) == digest
    descriptor = export / '3d-assets/croisement03-southeast-stone-wall/asset.json'
    wall = json.loads(descriptor.read_text())
    origin = wall['source_origin_scene']
    transform = dict(dx=origin[0], dy=-origin[1] * math.sin(math.radians(35)),
                     dz=origin[2] * math.cos(math.radians(35)), rot_deg=0)
    assert {part['source_obstacle'] for part in wall['parts']} == set(range(87, 94))
    errors = []
    rows = []
    for part in wall['parts']:
        index = part['source_obstacle']
        prior = next(row for row in mapping['rows'] if row['native_obstacle'] == index)
        path = ROOT / 'level-editor/library/3d-assets/croisement03' / prior['asset_id'] / 'asset.json'
        assert sha(path) == prior['descriptor_sha256']
        old = json.loads(path.read_text())
        assert len(old['parts']) == 1
        before = old['parts'][0]['obstacle_local_game']
        after = part['obstacle_local_game']
        assert {key: value for key, value in before.items() if key != 'points'} == {
            key: value for key, value in after.items() if key != 'points'}
        old_transform = prior['placement']['transform']
        assert old_transform['rot_deg'] == 0
        local_errors = []
        for a, b in zip(before['points'], after['points'], strict=True):
            for key, offset in [('x', 'dx'), ('y', 'dy'), ('z_bottom', 'dz'), ('z_top', 'dz')]:
                local_errors.append(abs(a[key] + old_transform[offset] - b[key] - transform[offset]))
        assert max(local_errors) < 1e-9
        errors.extend(local_errors)
        rows.append(dict(source_obstacle=index, max_world_error=max(local_errors),
                         flags_exact=True, points=len(before['points']),
                         original_gameplay=old['gameplay']))
    report = dict(status='PASS all seven world obstacle records; private preflight only',
                  private_descriptor_sha256=sha(descriptor), live_map_sha256=sha(live),
                  proposed_transform=transform, maximum_world_error=max(errors), parts=rows,
                  remaining=['Convert private raw export to the canonical nested library format.',
                             'Preserve the recorded original gameplay metadata during that conversion.',
                             'Complete browser and full-scene context review before coordinated publication.'])
    (RUN / 'wall-world-obstacle-proof.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'parts': len(rows), 'maximum_world_error': max(errors)}))


if __name__ == '__main__':
    main()
