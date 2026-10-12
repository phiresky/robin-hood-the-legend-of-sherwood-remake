"""Read-only regional terrain measurements for the complete northern wood group."""
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
from evidence_io import sha

BASE = ROOT / 'level-editor/work/croisement01-refinement/restart2'
PLAN = BASE / 'northern-wood-construction-plan-v1'
BANK = BASE / 'tree05-regional-banks-v18/assets/croisement01-tree-05/model.blend'
BANK_SHA = '7926500e407950bd66b78d13e0ce65b3829dd2263d7db4ba5c57a7ece5dbc818'


def main():
    acquire(slots=2)
    assert shutil.disk_usage(ROOT).free >= 10 * 1024**3
    assert int(next(s.split()[1] for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))) * 1024 >= 6 * 1024**3
    assert sha(BANK) == BANK_SHA
    dest = PLAN / 'bank-ray-probe-v1.json'
    assert not dest.exists()
    bpy.ops.wm.open_mainfile(filepath=str(BANK))
    bpy.context.window.scene = bpy.data.scenes['Croisement01 Refinement']
    bpy.context.view_layer.update()
    banks = []
    for obj in bpy.context.scene.objects:
        node = obj.get('source_node')
        if obj.type != 'MESH' or node not in {'ground', *[f'building-{n:03}' for n in range(10)]}:
            continue
        points = [obj.matrix_world @ v.co for v in obj.data.vertices]
        banks.append((node, BVHTree.FromPolygons(points, [list(p.vertices) for p in obj.data.polygons])))
    assert banks
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    ray = Vector((0, -cosine, sine))
    result = []
    for record in json.loads((PLAN / 'plan.json').read_text())['records']:
        left, top, right, bottom = record['native_bounds']
        samples = []
        for x in (left, (left+right)/2, right):
            for y in (bottom-12, bottom, bottom+12):
                origin = Vector((x, -y/sine, 0)) + ray*5000
                hits = []
                for node, tree in banks:
                    point, normal, face, distance = tree.ray_cast(origin, -ray, 10000)
                    if point is not None:
                        hits.append({'node': node, 'world': list(point), 'normal': list(normal), 'source_ray_distance': distance})
                samples.append({'source_position': [x, y], 'terrain_hits_front_to_back': sorted(hits, key=lambda h:h['source_ray_distance'])})
        result.append({'native_mask': record['native_mask'], 'samples': samples})
    dest.write_text(json.dumps({'status':'MEASURED_TERRAIN_CANDIDATES_NOT_ROOT_CONTACT_APPROVAL', 'bank_sha256': BANK_SHA, 'plan_sha256':sha(PLAN/'plan.json'), 'records':result, 'limitations':['Samples surround the native mask bottom, not an asserted physical root.', 'Select the source-supported contact after comparison with original artwork; retain multiple terrain hits explicitly.', 'No geometry or canonical changes.']}, indent=2)+'\n')
    print(dest, flush=True)


if __name__ == '__main__':
    main()
