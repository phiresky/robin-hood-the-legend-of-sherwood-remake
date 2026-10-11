"""Measure restored bark coverage and saved wood topology independently of projection masks."""
import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha

R = ROOT / 'level-editor/work/croisement01-refinement/restart2'
acquire(slots=2)
parser = argparse.ArgumentParser(); parser.add_argument('--revision', type=int, default=10)
args = parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
worker = R / f'tree06-v{args.revision}/assets/croisement01-tree-06'
bpy.ops.wm.open_mainfile(filepath=str(worker / 'model.blend'))
objects = [o for o in bpy.context.scene.objects if o.type == 'MESH' and o.get('source_node') == 'scenery-tree06-wood']
assert len(objects) == 1
obj = objects[0]
mesh = obj.data
tree = BVHTree.FromPolygons([obj.matrix_world @ v.co for v in mesh.vertices], [tuple(f.vertices) for f in mesh.polygons])
bm = bmesh.new(); bm.from_mesh(mesh)
nonmanifold = sum(not e.is_manifold for e in bm.edges)
remaining = set(bm.verts); components = []; vertex_component = {}
while remaining:
    stack = [remaining.pop()]; count = 0
    while stack:
        v = stack.pop(); count += 1; vertex_component[v.index] = len(components)
        for e in v.link_edges:
            other = e.other_vert(v)
            if other in remaining:
                remaining.remove(other); stack.append(other)
    components.append(count)
bm.free()
native = json.loads((R.parent / 'baseline/masks/manifest.json').read_text())
row = next(r for r in native['masks'] if r['index'] == 6)
left, top = row['box_top_left']
mask = np.asarray(Image.open(R / 'tree06-wood-source-v3/wood-domain.png').convert('L')) > 0
old = np.asarray(Image.open(R / 'tree06-wood-source-v2/wood-domain.png').convert('L')) > 0
pad = np.pad(mask, 1)
interior = np.logical_and.reduce([pad[i:i+mask.shape[0], j:j+mask.shape[1]] for i in range(3) for j in range(3)])
s, c = math.sin(math.radians(35)), math.cos(math.radians(35))
ray = Vector((0, -c, s)); misses = []; restored_misses = []; component_hits = {}
for y, x in zip(*np.nonzero(mask)):
    origin = Vector((float(left+x)+.5, -(float(top+y)+.5)/s, 0)) + ray*5000
    hit, _, face, _ = tree.ray_cast(origin, -ray, 10000)
    if hit is None:
        misses.append([int(x), int(y)])
        if not old[y, x]: restored_misses.append([int(x), int(y)])
    else:
        component = vertex_component[mesh.polygons[face].vertices[0]]
        component_hits[component] = component_hits.get(component, 0) + 1
report = dict(status='MEASURED_NOT_APPROVED', model_sha256=sha(worker/'model.blend'),
              source_domain_sha256=sha(R/'tree06-wood-source-v3/wood-domain.png'),
              source_pixels=int(mask.sum()), restored_pixels=int((mask & ~old).sum()),
              missed_source_pixels=misses, missed_restored_pixels=restored_misses,
              missed_interior_pixels=[p for p in misses if interior[p[1], p[0]]],
              topology=dict(vertices=len(mesh.vertices), faces=len(mesh.polygons),
                            nonmanifold_edges=nonmanifold, component_vertices=sorted(components, reverse=True),
                            components=[dict(vertices=n, source_hits=component_hits.get(i, 0)) for i, n in enumerate(components)]),
              limitations=['Native ray coverage only; no source color or bank occlusion certification.',
                           'Crown, branch intersections and current bank contact need separate review.'])
(worker/'inspection/restored-source-coverage.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(dict(source=int(mask.sum()), misses=len(misses), restored_misses=len(restored_misses), topology=report['topology'])))
