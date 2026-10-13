"""Bind recorded airborne poses and exact animation frames to a verified map ZIP."""
import argparse
from collections import defaultdict
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import zipfile

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    parser.add_argument('descriptor')
    parser.add_argument('archive', type=Path)
    parser.add_argument('sprite_bank', type=Path)
    parser.add_argument('rhs', type=Path)
    parser.add_argument('--profile', default='Robin des bois')
    args = parser.parse_args()
    if Path(args.descriptor).name != args.descriptor:
        raise ValueError('Descriptor must be a filename in the trace directory')
    report = json.loads((args.trace / 'actor-supported-floor-click-report.json').read_text())
    if not report['complete'] or not all(row['passed'] for row in report['results']):
        raise ValueError('Complete the supported-click traversal audit first')
    contact = next(row for row in json.loads(
        (args.trace / 'observed-solid-contact.json').read_text())['hits']
        if row['file'] == args.descriptor)
    route = next(row for row in report['results']
                 if row['file'] == args.descriptor and row['probe']['line'] == contact['line']
                 and row['arrival']['trajectory'][contact['frame']]['position'] == contact['position'])
    geometry = json.loads((args.trace / args.descriptor).read_text())['asset_geometry']
    with zipfile.ZipFile(args.archive) as archive:
        levels = [name for name in archive.namelist() if name.endswith('.level.json')]
        if len(levels) != 1:
            raise ValueError('Expected exactly one archived level descriptor')
        archived = json.loads(archive.read(levels[0]))['asset_geometry']
    expected = deepcopy(geometry)
    expected.pop('warnings', None)
    archived.pop('warnings', None)
    if expected != archived:
        raise ValueError('Archived gameplay differs from the recorded traversal')
    manifest = json.loads((args.sprite_bank / 'manifest.json').read_text())
    with args.rhs.open('rb') as source:
        signature = struct.unpack('<I', source.read(4))[0]
    if signature != manifest['signature']:
        raise ValueError('Sprite manifest and movement profile signatures differ')
    profile = next(row for row in manifest['profiles'] if row['name'] == args.profile)
    pixel_format = manifest.get('pixel_format', 'legacy_color_keys')
    if pixel_format not in ('rgba', 'legacy_color_keys'):
        raise ValueError(f'Unknown sprite pixel format: {pixel_format}')
    groups = defaultdict(list)
    for frame, pose in enumerate(route['arrival']['trajectory']):
        if pose['action'] not in ('JumpingLong', 'JumpingLongSword', 'JumpingUp', 'JumpingDown'):
            continue
        if pose['sprite_alternate_profile']:
            raise ValueError('Supply an explicit alternate profile before capturing this route')
        row = profile['rows'][pose['sprite_row']]
        if row['action'] != pose['action']:
            raise ValueError('Recorded animation row does not match the supplied manifest')
        x, y, z = pose['position']
        if pose['posture'] not in ('Flying', 'Upright') or not isinstance(pose['sector'], int):
            raise ValueError('Capture requires a supported recorded posture and sector')
        groups[(pose['sprite_row'], pose['sprite_frame'])].append({
            'name': f'frame-{frame:03d}', 'map': [x, y - z], 'height': z,
            'layer': pose['layer'], 'sector': pose['sector'], 'flying': pose['posture'] == 'Flying',
        })
    if not groups:
        raise ValueError('No airborne sprite frames in the selected route')
    output = Path(tempfile.mkdtemp(prefix='roof-character-captures-', dir=args.trace.parent))
    for (row_index, frame_index), probes in groups.items():
        row = profile['rows'][row_index]
        frame = row['frames'][frame_index]
        image = (args.sprite_bank / row['path'] / frame['file']).resolve(strict=True)
        if not image.is_relative_to(args.sprite_bank.resolve()):
            raise ValueError('Sprite frame escapes its bank')
        with Image.open(image) as decoded:
            width, height = decoded.size
        offset = [frame[f'offset_{axis}'] - row[f'hotspot_{axis}'] for axis in ('x', 'y')]
        if any(value != int(value) or not -32768 <= value <= 32767 for value in offset):
            raise ValueError('Capture requires representable sprite offsets')
        config = {
            'archive': str(args.archive.resolve()), 'sprite_sheet': str(image),
            'sprite_rect': [0, 0, width, height], 'sprite_offset': list(map(int, offset)),
            'legacy_color_keys': pixel_format == 'legacy_color_keys',
            'output': str(output.resolve() / 'images'), 'probes': probes,
        }
        (output / f'row-{row_index}-frame-{frame_index}.json').write_text(json.dumps(config))
    (output / 'preparation.json').write_text(json.dumps({
        'scope': 'recorded-airborne-sprite-poses-not-continuous-animation',
        'trace': str(args.trace.resolve()), 'file': args.descriptor, 'probe': route['probe'],
        'contact': contact, 'sprite_signature': signature,
        'archive_sha256': hashlib.sha256(args.archive.read_bytes()).hexdigest(),
        'configurations': len(groups), 'poses': sum(map(len, groups.values())),
    }))
    print(output)


if __name__ == '__main__':
    main()
