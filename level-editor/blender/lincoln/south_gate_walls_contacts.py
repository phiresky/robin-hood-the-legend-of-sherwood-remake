"""Footing contact check against the integrated neighbours (Blender, read-only).

  SOUTH_GATE_ROUND=round-2 /usr/bin/blender --background --threads 2 --python-exit-code 1 \
    --python level-editor/blender/lincoln/south_gate_walls_contacts.py -- --asset <id>

For every owned mesh, samples the bottom boundary vertices and casts a vertical
ray downward from 40 units above a point 2 units south of each footing vertex,
skipping owned meshes. The first neighbour surface below is the ground in front
of (or under) the footing; footing z minus that surface z > 4.5 means the
footing floats above the neighbour; no hit means open air (e.g. the ravine). Writes
<workspace>/inspection/contacts.json.
"""
import argparse
import json
import os
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
    from mathutils import Vector
    bpy.ops.wm.open_mainfile(filepath=str(ws / 'model.blend'))
    objs = [o for o in bpy.data.collections['lincoln Working'].all_objects if o.type == 'MESH']
    owned = [o for o in objs if o.get('asset_group') == args.asset]
    owned_names = {o.name for o in owned}
    working = {o.name for o in objs if not o.hide_render}
    dg = bpy.context.evaluated_depsgraph_get()
    scene = bpy.context.scene
    out = {}
    for o in owned:
        verts = [to_native(tuple(o.matrix_world @ v.co)) for v in o.data.vertices]
        zmin = min(v[2] for v in verts)
        foot = [v for v in verts if v[2] < zmin + 60 and v[2] < 222]
        rows = []
        for x, y, z in foot:
            # only camera-facing footings: probe 2 units south of the vertex
            probe = (x, y + 2.0, z + 40.0)
            origin = Vector(to_world(probe))
            hit_down = None
            o_ = origin
            for _ in range(6):
                hit, loc, nrm, idx, ob, _m = scene.ray_cast(dg, o_, Vector((0, 0, -1)))
                if not hit:
                    break
                if ob.name not in working or ob.name in owned_names:
                    o_ = loc + Vector((0, 0, -0.01))
                    continue
                hit_down = (round(z - to_native(tuple(loc))[2], 2), ob.get('asset_group') or ob.get('source_node'))
                break
            rows.append({'x': round(x, 1), 'y': round(y, 1), 'z': round(z, 1), 'gap_below_front': hit_down})
        floating = [r for r in rows if r['gap_below_front'] and r['gap_below_front'][0] > 4.5]
        out[o['source_node']] = {'footing_vertices': len(rows),
                                 'floating_over_neighbour': len(floating),
                                 'open_air': sum(1 for r in rows if r['gap_below_front'] is None),
                                 'worst': sorted(floating, key=lambda r: -r['gap_below_front'][0])[:5]}
    (ws / 'inspection').mkdir(exist_ok=True)
    (ws / 'inspection/contacts.json').write_text(json.dumps(out, indent=1) + '\n')
    print('CONTACTS', args.asset, json.dumps({k: (v['footing_vertices'], v['floating_over_neighbour'], v['open_air'])
                                              for k, v in out.items()}))


if __name__ == '__main__':
    main()
