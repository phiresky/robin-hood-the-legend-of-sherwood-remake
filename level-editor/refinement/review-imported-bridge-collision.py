"""Render mesh and compiled authoring solids for the pinned bridge candidate.

Run from level-editor with a staged candidate directory as the only argument.
This geometry review does not replace rendered in-game actor verification.
"""
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from matplotlib.collections import PolyCollection
import numpy as np

candidate = Path(sys.argv[1])
review = json.loads(Path('refinement/catalogs/sketchfab-long-wood-bridge-deck-review.json').read_text())
model = Path('library/3d-assets/sketchfab/sketchfab-long-wood-bridge/model.glb').read_bytes()
assert hashlib.sha256(model).hexdigest() == review['modelSha256']
length = struct.unpack_from('<I', model, 12)[0]
gltf = json.loads(model[20:20 + length])
binary = model[28 + length:]


def accessor(index):
    entry = gltf['accessors'][index]
    view = gltf['bufferViews'][entry['bufferView']]
    width = {'VEC3': 3, 'SCALAR': 1}[entry['type']]
    dtype = {5123: '<u2', 5125: '<u4', 5126: '<f4'}[entry['componentType']]
    assert view.get('byteStride', np.dtype(dtype).itemsize * width) == np.dtype(dtype).itemsize * width
    return np.frombuffer(binary, dtype=dtype, count=entry['count'] * width,
                         offset=view.get('byteOffset', 0) + entry.get('byteOffset', 0)).reshape(-1, width)


node = next(n for n in gltf['nodes'] if n.get('name') == review['node'])
assert not any(k in node for k in ('matrix', 'translation', 'rotation', 'scale'))
primitive = gltf['meshes'][node['mesh']]['primitives'][review['primitive']]
vertices = accessor(primitive['attributes']['POSITION']).astype(float)
vertices *= [1, -math.sin(math.radians(35)), math.cos(math.radians(35))]
triangles = vertices[accessor(primitive['indices']).reshape(-1, 3)]
gameplay = json.loads((candidate / 'candidate.gameplay.json').read_text())
solids = []
for volume in gameplay['volumes']:
    points = volume['shape']['points']
    bottom = np.array([[p['x'], p['y'], p['z_bottom']] for p in points])
    top = np.array([[p['x'], p['y'], p['z_top']] for p in points])
    solids.extend([bottom, top])
    solids.extend(np.array([bottom[i], bottom[(i + 1) % len(points)],
                            top[(i + 1) % len(points)], top[i]]) for i in range(len(points)))
surfaces = [np.array([[*p, h] for p, h in zip(s['polygon'], s['height'])])
            for s in gameplay['surfaces']]

fig, axes = plt.subplots(2, 3, figsize=(16, 9), layout='constrained')
for column, (dimensions, title) in enumerate([((0, 1), 'Plan'), ((1, 2), 'Length / height'), ((0, 2), 'Width / height')]):
    for row, polygons in enumerate((triangles, solids)):
        ax = axes[row, column]
        ax.add_collection(PolyCollection([p[:, dimensions] for p in polygons],
                                        facecolor='#826540' if row == 0 else '#d18b45',
                                        edgecolor='none', alpha=0.75))
        ax.add_collection(PolyCollection([p[:, dimensions] for p in surfaces],
                                        facecolor='none', edgecolor='#008040', linewidth=0.8))
        ax.autoscale_view()
        ax.set_aspect('equal')
        ax.set_title(('Mesh' if row == 0 else 'Authored collision') + ': ' + title)
        ax.set_xlabel('XYZ'[dimensions[0]] + ' (game units)')
        ax.set_ylabel('XYZ'[dimensions[1]] + ' (game units)')
        ax.grid(alpha=0.15)
        if dimensions[1] == 2:
            ax.axhline(80, color='#a02050', linestyle='--', linewidth=0.8)
            ax.axhline(0, color='black', linewidth=0.5)
fig.suptitle('Bridge geometry review — green: walking deck; dashed: 80-unit upright clearance from foundation\n'
             'Physical solids remain separate from navigation headroom. No texture/actor compositing is tested here.')
output = candidate / 'collision-review.png'
fig.savefig(output, dpi=150)
plt.close(fig)
print(output)
