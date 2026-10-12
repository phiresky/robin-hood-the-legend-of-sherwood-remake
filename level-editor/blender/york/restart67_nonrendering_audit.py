"""Reconcile nonvisual native volumes through current placed asset descriptors."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
LIB = ROOT / 'level-editor/library'
OUT = ROOT / 'level-editor/work/active/york/restart2/restart67-nonrendering-audit-v1'


def read(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    paths = [LIB / 'scenes/york.rhlos-map.json',
             ROOT / 'level-editor/refinement/catalogs/york.json',
             ROOT / 'level-editor/work/active/york/baseline/york.rhp.json']
    pins = {str(p.relative_to(ROOT)): sha(p) for p in paths}
    scene, catalog, level = map(read, paths)
    rows = []
    for record in catalog['nonrendering_sources']:
        number = record['obstacle']
        expected = level['sight_obstacles'][number]
        matches = []
        for source in scene['assetSources']:
            path = LIB / source['descriptor']
            descriptor = read(path)
            for volume in descriptor.get('gameplay', {}).get('volumes', []):
                if volume['id'] != f'gameplay-volume-{number}':
                    continue
                assert sha(path) == source['descriptor_sha256'], path
                pins[str(path.relative_to(ROOT))] = sha(path)
                shape = volume['shape']
                for key in ('opaque', 'solid', 'mouse', 'show_shadow_polygon', 'default_material'):
                    assert shape[key] == expected[key], (number, key)
                for placement in scene['placements']:
                    if source['id'] not in placement['assets']:
                        continue
                    transform = placement['transform']
                    assert transform['rot_deg'] == 0, 'Rotated placement requires explicit transform audit'
                    assert len(shape['points']) == len(expected['points'])
                    error = max(abs(local[key] + transform[axis] - native[key])
                                for local, native in zip(shape['points'], expected['points'])
                                for key, axis in [('x', 'dx'), ('y', 'dy'), ('z_bottom', 'dz'), ('z_top', 'dz')])
                    assert error < 0.001, (number, error)
                    matches.append({'asset': source['id'], 'placement': placement['id'],
                                    'bound_node': volume['node'], 'max_world_coordinate_error': error,
                                    'descriptor': source['descriptor'], 'descriptor_sha256': sha(path)})
        assert len(matches) == 1, (number, matches)
        rows.append({'native_obstacle': number, 'catalog_owner': record['owner'],
                     'classification': 'INTENTIONALLY_NONVISUAL_RETAINED_AS_PLACED_GAMEPLAY_VOLUME',
                     'catalog_reason': record['reason'], 'installed': matches[0]})
    for path, expected in pins.items():
        assert sha(ROOT / path) == expected, 'Concurrent input drift: ' + path
    OUT.mkdir(parents=True, exist_ok=False)
    (OUT / 'report.json').write_text(json.dumps({
        'status': 'PASS_ALL_NINE_NATIVE_NONVISUAL_VOLUMES_PRESENT', 'rows': rows, 'pins': pins,
        'canonical_writes': False,
        'limits': 'Current descriptor ownership, flags and placed world shape only. Does not certify compiled navigation, appearance or mission runtime.'
    }, indent=2) + '\n')
    print(OUT / 'report.json')


if __name__ == '__main__':
    main()
