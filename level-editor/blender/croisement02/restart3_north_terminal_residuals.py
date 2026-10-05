"""Measure remaining native pixel-center boundary errors without changing models."""
import collections
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
from scipy.ndimage import distance_transform_edt, label


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    worker = Path(sys.argv[1])
    source = worker / 'source-front-v1/rays.json'
    rows = json.loads(source.read_text())['rows']
    result = {
        'model_sha256': sha(worker / 'worker.blend'),
        'source_rays_sha256': sha(source),
        'method': 'Connected components and Euclidean distance on original native pixel-center classifications.',
        'limitations': [
            'Distance to a non-error pixel measures the radius of each error patch, not its total width.',
            'This does not prove subpixel coverage or identify source ownership independently.',
            'Unknown dark outlines remain unassigned; they are not automatically shadows or geometry.',
        ],
        'kinds': {},
    }
    for kind in ['assigned-gray-or-foreign', 'assigned-no-geometry', 'unassigned']:
        mask = np.zeros((142, 252), dtype=bool)
        selected = [row for row in rows if row[3] == kind]
        for row in selected:
            mask[row[1], row[0]] = True
        distances = distance_transform_edt(mask)
        groups, count = label(mask)
        components = []
        for index in range(1, count + 1):
            yy, xx = np.where(groups == index)
            components.append({
                'pixels': len(xx),
                'box': [int(xx.min()), int(yy.min()), int(xx.max() + 1), int(yy.max() + 1)],
                'maximum_interior_radius_pixels': float(distances[groups == index].max()),
            })
        result['kinds'][kind] = {
            'pixels': int(mask.sum()),
            'maximum_interior_radius_pixels': float(distances.max()),
            'interior_distance2_by_role': dict(collections.Counter(row[2] for row in selected if row[4])),
            'components': sorted(components, key=lambda row: -row['pixels']),
        }
    output = worker / 'source-front-v1/residual-components.json'
    if output.exists():
        raise RuntimeError(f'Refusing to overwrite frozen diagnostic: {output}')
    output.write_text(json.dumps(result, indent=2) + '\n')
    print(output)


if __name__ == '__main__':
    main()
