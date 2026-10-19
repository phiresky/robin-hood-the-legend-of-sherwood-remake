"""Extract exact editor archive bytes for the native map audit tools."""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import zipfile


def prepare(archive):
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
    output = Path(tempfile.mkdtemp(prefix='exact-map-archive-', dir=archive.parent.parent))
    (output / 'archive.level.json').write_bytes(level_bytes)
    (output / 'archive.rhlos-map.json').write_bytes(scene_bytes)
    (output / 'diagnostics.json').write_text(json.dumps({
        'scope': 'static-geometry-only-not-gameplay-parity',
        'complete': True,
        'archive': str(archive),
        'archiveSha256': archive_hash,
        'levelSha256': hashlib.sha256(level_bytes).hexdigest(),
        'results': [{'map': scene['map'], 'file': 'archive.level.json'}],
    }, indent=2))
    return output


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive')
    print(prepare(parser.parse_args().archive))
