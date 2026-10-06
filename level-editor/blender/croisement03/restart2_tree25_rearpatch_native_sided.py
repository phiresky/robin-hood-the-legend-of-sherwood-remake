"""Compare native-camera physical first hits before and after private rear edits."""
import hashlib
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire, release


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def evaluate(model, view):
    bpy.ops.wm.open_mainfile(filepath=str(model))
    obj = next(o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == 'croisement03-tree-25')
    mesh = obj.data; mesh.calc_loop_triangles()
    vertices = [obj.matrix_world @ v.co for v in mesh.vertices]
    triangles = list(mesh.loop_triangles)
    bvh = BVHTree.FromPolygons(vertices, [list(t.vertices) for t in triangles], all_triangles=True)
    atlas = {}
    for slot, mat in enumerate(mesh.materials):
        if not mat or not mat.get('foliage_physical_opacity'):
            continue
        node = next(n for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image)
        values = np.empty(len(node.image.pixels), np.float32); node.image.pixels.foreach_get(values)
        atlas[slot] = (values.reshape(node.image.size[1], node.image.size[0], 4)[...,3],
                       mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map],
                       mat.get('foliage_card_sides') == 'paired-one-sided')
    camera = Matrix(view['camera_matrix_world'])
    direction = camera.to_3x3() @ Vector((0,0,-1)); direction.normalize()
    scale = view['ortho_scale']; result = np.full((384,384,3), -1, np.int32)
    limited = 0
    for y in range(384):
        for x in range(384):
            origin = camera @ Vector((((x+.5)/384-.5)*scale, (.5-(y+.5)/384)*scale, 0))
            for step in range(256):
                p, normal, tid, distance = bvh.ray_cast(origin, direction)
                if p is None:
                    break
                tri = triangles[tid]; slot = mesh.polygons[tri.polygon_index].material_index
                if slot not in atlas:
                    result[y,x] = (slot,0,0); break
                alpha, uv, one_sided = atlas[slot]
                if one_sided and normal.dot(direction) >= 0:
                    origin = p + direction*.002
                    continue
                mapped = barycentric_transform(p, *[vertices[v] for v in tri.vertices],
                    *[Vector((*uv.data[i].uv,0)) for i in tri.loops])
                tx = min(alpha.shape[1]-1, int((mapped.x % 1)*alpha.shape[1]))
                ty = min(alpha.shape[0]-1, int((mapped.y % 1)*alpha.shape[0]))
                if alpha[ty,tx] >= .5:
                    result[y,x] = (slot,tx,ty); break
                origin = p + direction*.002
            else:
                limited += 1
        if y % 96 == 0:
            print(model.parent.name, 'row', y, flush=True)
    return result, limited


def main():
    e = ROOT / 'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
    out = e / 'rearpatch-geometry-v1'; report = json.loads((out / 'construction.json').read_text())
    before, after = e / 'native-rgb-control-v1/worker.blend', out / 'worker.blend'
    assert sha(before) == report['source_model_sha256'] and sha(after) == report['model_sha256']
    assert not (out / 'native-first-hit-sided.json').exists()
    view = json.loads((e / 'views-grid8-v4.json').read_text())['views'][0]
    acquire()
    try:
        a, limited_a = evaluate(before, view); b, limited_b = evaluate(after, view)
        changed = np.any(a != b, axis=2)
        coverage = (a[...,0] >= 0) != (b[...,0] >= 0)
        ownership = a[...,0] != b[...,0]
        rows = [{'pixel':[int(x),int(y)], 'before':a[y,x].tolist(), 'after':b[y,x].tolist()}
                for y,x in zip(*np.where(changed))]
        result = dict(status='PASS' if not rows and limited_a == limited_b == 0 else 'HOLD differences require classification',
            model_sha256=sha(after), baseline_sha256=sha(before), native_view_index=0,
            sampled_pixels=384*384, depth_limits=[limited_a,limited_b],
            changed_first_hit_samples=int(changed.sum()), changed_coverage_pixels=int(coverage.sum()),
            changed_material_owner_pixels=int(ownership.sum()), differences=rows,
            method='Every native review-camera pixel centre; nearest physical alpha threshold .5 and authored paired-one-sided culling; 0.002 world-unit ray advancement. Compares first material and exact atlas texel, not antialiasing or lighting.')
        (out / 'native-first-hit-sided.json').write_text(json.dumps(result,indent=2)+'\n')
        print({k:v for k,v in result.items() if k != 'differences'})
    finally:
        release()


if __name__ == '__main__':
    main()
