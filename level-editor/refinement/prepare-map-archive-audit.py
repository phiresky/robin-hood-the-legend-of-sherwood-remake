"""Extract exact editor archive bytes for the native map audit tools."""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import zipfile


def prepare(archive, approaches=None, approach_file=None):
    archive = Path(archive).resolve(strict=True)
    archive_bytes = archive.read_bytes()
    archive_hash = hashlib.sha256(archive_bytes).hexdigest()
    evidence = json.loads((archive.parent / 'browser-roundtrip-evidence.json').read_bytes())
    assert evidence['browserRoundtrip'].startswith('PASS ')
    assert evidence['zipSha256'] == archive_hash and evidence['bytes'] == len(archive_bytes)
    with zipfile.ZipFile(archive) as zipped:
        names = zipped.namelist()
        assert len(set(names)) == len(names), 'Duplicate archive entries'
        levels = [name for name in names if name.startswith('Data/Levels/') and name.endswith('.level.json')]
        scenes = [name for name in names if name.startswith('editor/') and name.endswith('.rhlos-map.json')]
        assert len(levels) == len(scenes) == 1
        level_bytes, scene_bytes = zipped.read(levels[0]), zipped.read(scenes[0])
    level, scene = json.loads(level_bytes), json.loads(scene_bytes)
    assert level.get('spawn_points') == [] and 'spawn_player' not in level
    assert not scene.get('mission')
    assert scene['map'] == level['title']
    assert scene.get('assetSources')
    result = {'map': scene['map'], 'file': 'archive.level.json'}
    if approaches is not None:
        assert approach_file, 'Select the placement file whose approaches are being copied'
        source = Path(approaches).resolve(strict=True)
        manifest = json.loads((source / 'diagnostics.json').read_bytes())
        assert manifest['complete'] is True
        matches = [row for row in manifest['results'] if row['file'] == approach_file]
        assert len(matches) == 1
        probes = matches[0]['building_approaches']
        assert probes, 'No selected entrance approaches'
        original = json.loads((source / approach_file).read_bytes())
        for key in ('buildings', 'motion_data'):
            assert original['asset_geometry'][key] == level['asset_geometry'][key], \
                f'Archive {key} differs from the placement audit'
        result['building_approaches'] = probes
        result['approach_source'] = str(source / approach_file)
    output = Path(tempfile.mkdtemp(prefix='exact-map-archive-', dir=archive.parent.parent))
    (output / 'archive.level.json').write_bytes(level_bytes)
    (output / 'archive.rhlos-map.json').write_bytes(scene_bytes)
    (output / 'diagnostics.json').write_text(json.dumps({
        'scope': 'static-geometry-only-not-gameplay-parity',
        'complete': True,
        'archive': str(archive),
        'archiveSha256': archive_hash,
        'levelSha256': hashlib.sha256(level_bytes).hexdigest(),
        'results': [result],
    }, indent=2))
    return output


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive')
    parser.add_argument('--approaches', help='Completed placement audit directory')
    parser.add_argument('--approach-file', help='One level filename from the placement audit')
    args = parser.parse_args()
    assert bool(args.approaches) == bool(args.approach_file)
    print(prepare(args.archive, args.approaches, args.approach_file))
