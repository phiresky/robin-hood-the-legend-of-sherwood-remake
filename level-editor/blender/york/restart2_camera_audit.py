"""Check native-first York review manifests and add labeled companion sheets."""
import argparse
import hashlib
import json
import math
from pathlib import Path


def audit_manifest(path):
    data = json.loads(path.read_text())
    views = data['views']
    first = views[0]
    matrix = first['camera_matrix_world']
    angle = math.radians(35)
    expected = ((1, 0, 0), (0, math.sin(angle), -math.cos(angle)),
                (0, math.cos(angle), math.sin(angle)))
    error = max(abs(matrix[r][c] - expected[r][c]) for r in range(3) for c in range(3))
    if first['index'] != 0 or first['azimuth_degrees'] != 0 or error > 1e-6:
        raise ValueError(f'Non-native first view: {path}')
    if first['ortho_scale'] <= 0 or data['layout'] != {'columns': 4, 'rows': 2}:
        raise ValueError(f'Invalid orthographic layout: {path}')
    return {'manifest': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
            'view_0': 'Native game camera: orthographic, yaw 0 degrees, elevation 35 degrees',
            'rotation_max_error': error}


def labeled_copy(source, destination):
    from PIL import Image, ImageDraw
    with Image.open(source) as original:
        sheet = original.convert('RGBA')
    width, height = sheet.width // 4, sheet.height // 2
    draw = ImageDraw.Draw(sheet)
    for index in range(8):
        x, y = index % 4 * width, index // 4 * height
        label = '0 - Native game camera (ortho, 35 deg)' if index == 0 else f'{index} - Orbit {index * 45} deg'
        draw.rectangle((x, y, x + width - 1, y + 19), fill=(12, 16, 20, 240))
        draw.text((x + 6, y + 4), label, fill='white')
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise FileExistsError(destination)
    sheet.save(destination)


def label_packet(directory):
    directory = Path(directory)
    report = audit_manifest(directory / 'views.json')
    for name in ('solid', 'textured', 'known'):
        source = directory / f'{name}.png'
        if source.exists():
            labeled_copy(source, directory / f'{name}-native-labeled.png')
    (directory / 'native-camera-audit.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    args.output.mkdir(parents=True)
    rows = []
    for manifest in sorted(args.root.rglob('views.json')):
        data = json.loads(manifest.read_text())
        if not isinstance(data.get('views'), list) or not data['views'] or 'camera_matrix_world' not in data['views'][0]:
            continue
        rows.append(audit_manifest(manifest))
    result = {'scope': 'Manifest and frozen-renderer camera audit; geometry and source ownership remain separate.',
              'packets': rows, 'count': len(rows), 'native_first': True}
    (args.output / 'audit.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'manifests': len(rows), 'native_first': True}))


if __name__ == '__main__':
    main()
