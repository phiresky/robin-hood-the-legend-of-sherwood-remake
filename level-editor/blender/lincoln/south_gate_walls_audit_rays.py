"""Source-camera first-hit map for one south-gate lane workspace (Blender).

  /usr/bin/blender --background --threads 2 --python-exit-code 1 \
    --python level-editor/blender/lincoln/south_gate_walls_audit_rays.py -- --asset <id>

Opens the saved <workspace>/model.blend read-only and casts one ray per source
pixel (35 degree oblique source camera) over the owned assets' projected bounds.
Writes <workspace>/inspection/coverage-hits.npz: first-hit class per pixel
(0 nothing, 1 owned mesh, 2 other mesh) and the owned node / foreign group index. This
domain is geometric and independent of the acceptance masks; the offline
south_gate_walls_audit.py compares it with the native masks and the artwork.
"""
import os
import argparse
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from south_gate_walls_geometry import to_world, to_native  # noqa: E402

R = HERE.parents[2] / 'level-editor/work/needs-review/lincoln-refinement'


def main():
    argv = sys.argv[sys.argv.index('--') + 1:]
    p = argparse.ArgumentParser()
    p.add_argument('--asset', required=True)
    args = p.parse_args(argv)
    ws = R / os.environ.get('SOUTH_GATE_ROUND', 'round-1') / 'assets' / args.asset
    from render_slots import acquire
    acquire()
    import bpy
    import numpy as np
    from mathutils import Vector
    bpy.ops.wm.open_mainfile(filepath=str(ws / 'model.blend'))
    objs = [o for o in bpy.data.collections['lincoln Working'].all_objects if o.type == 'MESH']
    owned = [o for o in objs if o.get('asset_group') == args.asset]
    nodes = sorted({o['source_node'] for o in owned})
    xs, ys = [], []
    for o in owned:
        for v in o.data.vertices:
            n = to_native(tuple(o.matrix_world @ v.co))
            xs.append(n[0])
            ys.append(n[1] - n[2])
    x0, x1 = int(min(xs)) - 6, int(max(xs)) + 7
    y0, y1 = int(min(ys)) - 6, int(max(ys)) + 7
    x0, y0 = max(0, x0), max(0, y0)
    x1, y1 = min(2944, x1), min(2176, y1)
    hidden = [o for o in objs if o.hide_render]
    dg = bpy.context.evaluated_depsgraph_get()
    scene = bpy.context.scene
    cls = np.zeros((y1 - y0, x1 - x0), np.uint8)
    node = np.full((y1 - y0, x1 - x0), 255, np.uint8)
    zhit = np.zeros((y1 - y0, x1 - x0), np.float32)
    owned_names = {o.name: nodes.index(o['source_node']) for o in owned}
    components = {n: sorted({o.get('projection_component') for o in owned if o['source_node'] == n},
                            key=lambda c: (c is None, c or '')) for n in nodes}
    comp_index = {o.name: components[o['source_node']].index(o.get('projection_component')) for o in owned}
    comp = np.full((y1 - y0, x1 - x0), 255, np.uint8)
    hidden_names = {o.name for o in hidden}
    working = {o.name for o in objs}  # baseline copies and helpers are ignored
    foreign = []
    top = 900.0
    for j, py in enumerate(range(y0, y1)):
        for i, px in enumerate(range(x0, x1)):
            origin = Vector(to_world((px + 0.5, py + 0.5 + top, top)))
            end = Vector(to_world((px + 0.5, py + 0.5 - 50.0, -50.0)))
            direction = (end - origin).normalized()
            o = origin
            for _ in range(8):
                hit, loc, nrm, idx, ob, _m = scene.ray_cast(dg, o, direction)
                if not hit:
                    break
                if (ob.name in hidden_names or ob.name not in working or ob.type != 'MESH'
                        or ob.get('source_node') is None):
                    o = loc + direction * 0.01
                    continue
                if ob.name in owned_names:
                    cls[j, i] = 1
                    node[j, i] = owned_names[ob.name]
                    comp[j, i] = comp_index[ob.name]
                else:
                    cls[j, i] = 2
                    group = ob.get('asset_group') or ob.get('source_node')
                    if group not in foreign:
                        foreign.append(group)
                    node[j, i] = min(254, foreign.index(group))
                zhit[j, i] = to_native(tuple(loc))[2]
                break
    (ws / 'inspection').mkdir(exist_ok=True)
    out = ws / 'inspection/coverage-hits.npz'
    np.savez_compressed(out, cls=cls, node=node, comp=comp, z=zhit, origin=np.array([x0, y0]))
    meta = {'asset_id': args.asset, 'nodes': nodes,
            'components': {n: c for n, c in components.items() if c != [None]}, 'foreign_groups': foreign, 'box': [x0, y0, x1, y1],
            'model_sha256': hashlib.sha256((ws / 'model.blend').read_bytes()).hexdigest(),
            'hidden_objects_skipped': sorted(hidden_names)}
    (ws / 'inspection/coverage-hits.json').write_text(json.dumps(meta, indent=1) + '\n')
    print('AUDIT-RAYS', args.asset, meta['box'], int((cls == 1).sum()))


if __name__ == '__main__':
    main()
