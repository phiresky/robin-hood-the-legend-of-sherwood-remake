"""Unite corrected branch sweeps and independently preserve their native first hits."""
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_bank_lofts_saved as guard

B = guard.B
S, C = math.sin(math.radians(35)), math.cos(math.radians(35))
RAY = Vector((0, -C, S))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bvh(obj):
    obj.data.calc_loop_triangles()
    return BVHTree.FromPolygons([obj.matrix_world @ v.co for v in obj.data.vertices],
                               [tuple(f.vertices) for f in obj.data.loop_triangles], all_triangles=True)


def hit(trees, source):
    x, y = source
    origin = Vector((x, -y / S, 0)) + RAY * 5000
    distances = [t.ray_cast(origin, -RAY)[3] for t in trees]
    return min((d for d in distances if d is not None), default=None)


def main():
    source = B / 'tree18-root-connectivity-saved-v1/worker.blend'
    geometry = B / 'tree18-root-connectivity-cpu-v1/geometry.json'
    out = B / 'tree18-united-wood-saved-v1'
    assert not out.exists()
    guard.resources(); guard.acquire(slots=2)
    try:
        guard.resources()
        pins = {str(p): sha(p) for p in [source, geometry]}
        bpy.ops.wm.open_mainfile(filepath=str(source))
        objects = sorted([o for o in bpy.context.scene.objects if o.type == 'MESH'], key=lambda o: o.name)
        assert len(objects) == 4
        before = [bvh(o) for o in objects]
        data = json.loads(geometry.read_text())
        points = np.array([list(o.matrix_world @ v.co) for o in objects for v in o.data.vertices])
        native = np.column_stack((points[:, 0], -points[:, 1] * S - points[:, 2] * C))
        lo, hi = np.floor(native.min(0)).astype(int), np.ceil(native.max(0)).astype(int)
        probes = [(x + .5, y + .5) for y in range(lo[1] - 1, hi[1] + 2) for x in range(lo[0] - 1, hi[0] + 2)]
        probes += [tuple(g['source_xy']) for p in data['parts'] for g in p['source_endpoint_guards']]
        baseline = [hit(before, p) for p in probes]
        target = next(o for o in objects if o.get('tree18_track') == 'main_lower')
        order = ['main_upper', 'left_fork', 'right_fork']
        for track in order:
            operand = next(o for o in objects if o.get('tree18_track') == track)
            bpy.context.view_layer.objects.active = target
            mod = target.modifiers.new('Join verified branch junction', 'BOOLEAN')
            mod.operation = 'UNION'; mod.solver = 'EXACT'; mod.object = operand
            bpy.ops.object.modifier_apply(modifier=mod.name)
            bpy.data.objects.remove(operand, do_unlink=True)
        target.name = 'Provisional Tree18 united wood'
        target['tree18_track'] = 'united_four_source_tracks'
        bm = bmesh.new(); bm.from_mesh(target.data)
        assert all(e.is_manifold for e in bm.edges)
        unseen = set(bm.verts); components = 0
        while unseen:
            pending = [unseen.pop()]; components += 1
            while pending:
                for edge in pending.pop().link_edges:
                    for vertex in edge.verts:
                        if vertex in unseen:
                            unseen.remove(vertex); pending.append(vertex)
        volume = float(bm.calc_volume()); bm.free()
        assert components == 1 and volume > 0
        out.mkdir()
        bpy.context.preferences.filepaths.save_version = 0
        model = out / 'worker.blend'
        bpy.ops.wm.save_as_mainfile(filepath=str(model))
        bpy.ops.wm.open_mainfile(filepath=str(model))
        after = [bvh(o) for o in bpy.context.scene.objects if o.type == 'MESH']
        assert len(after) == 1
        failures, errors = [], []
        for p, old in zip(probes, baseline):
            new = hit(after, p)
            if (old is None) != (new is None):
                failures.append({'xy': p, 'before': old, 'after': new})
            elif old is not None:
                errors.append(abs(old - new))
                if abs(old - new) > .002:
                    failures.append({'xy': p, 'before': old, 'after': new})
        report = {'status': 'PASS_SAVED_UNION_REQUIRES_VISUAL_CONTACT_REVIEW' if not failures else 'HOLD_FIRSTHIT_CHANGED',
                  'pins': pins, 'model_sha256': sha(model), 'connected_components': components,
                  'closed': True, 'volume': volume, 'native_firsthit_samples': len(probes),
                  'native_firsthit_max_error': max(errors, default=0), 'failures': failures,
                  'limits': ['Preserving prior aggregate source silhouette does not independently approve anatomy.',
                             'Ground placement and native display-priority behavior remain unresolved.',
                             'No crown, material assignments, gameplay or canonical changes.']}
        (out / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
        assert all(sha(Path(p)) == h for p, h in pins.items())
        assert model.stat().st_size < 8 * 1024**2
        assert not failures, report['status']
        print({k: report[k] for k in ['status', 'native_firsthit_samples', 'native_firsthit_max_error']})
    finally:
        guard.release()


if __name__ == '__main__':
    main()
