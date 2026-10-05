"""Compare saved crown frames with a tentative shrub domain, without runtime claims."""
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image


def main():
    root = Path(__file__).resolve().parents[2] / 'work/croisement03-refinement'
    output = root / 'restart2/east-shrub61-preflight'
    manifest_path = root / 'animation-references/manifest.json'
    manifest = json.loads(manifest_path.read_text())
    animation = next(row for row in manifest['animations'] if row['index'] == 1)
    domain_path = output / 'conservative-plant-domain.png'
    domain = np.array(Image.open(domain_path).convert('L')) > 0
    covered_count = np.zeros(domain.shape, dtype=np.uint16)
    frames = []
    for index, frame in enumerate(animation['frames']):
        path = Path(frame['image'])
        raw = path.read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        source_digest = hashlib.sha256(Path(frame['source']).read_bytes()).hexdigest()
        assert source_digest == frame['sha256'], (path, 'source frame changed')
        rgba = np.array(Image.open(path).convert('RGBA'))
        x, y, width, height = frame['bbox']
        assert rgba.shape[:2] == (height, width)
        assert x >= 0 and y >= 0 and y + height <= domain.shape[0] and x + width <= domain.shape[1]
        covered = np.zeros(domain.shape, dtype=bool)
        covered[y:y + height, x:x + width] = rgba[:, :, 3] > 0
        covered &= domain
        covered_count += covered
        frames.append({'index': index, 'saved_sha256': digest, 'source_sha256': source_digest,
                       'domain_overlap': int(covered.sum())})
    assert frames
    ever = covered_count > 0
    always = covered_count == len(frames)
    for name, pixels in [('crown-ever-overlap.png', ever), ('crown-always-overlap.png', always)]:
        Image.fromarray(pixels.astype(np.uint8) * 255).save(output / name)
    report = {
        'status': 'Saved-frame alpha diagnostic only; not runtime ordering or ownership approval',
        'manifest_sha256': hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
        'domain_sha256': hashlib.sha256(domain_path.read_bytes()).hexdigest(),
        'animation': animation['profile'], 'frames': frames,
        'domain_pixels': int(domain.sum()), 'ever_covered': int(ever.sum()),
        'always_covered': int(always.sum()),
        'intermittently_covered': int((ever & ~always).sum()),
        'never_covered': int((domain & ~ever).sum()),
        'limitations': [
            'Saved frame placement is a reference input; runtime phase and draw order are not simulated.',
            'Animated crown overlap does not authorize removing static shrub geometry or source pixels.',
            'The remaining domain still needs visual material ownership classification and joint geometry.',
        ],
    }
    (output / 'all-frame-overlap.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: value for key, value in report.items() if key != 'frames'}, indent=2))


if __name__ == '__main__':
    main()
