"""Private root-depth correction with fixed native projection and texture coordinates."""
import sys, json, hashlib
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN, COS, RAY
from render_slots import acquire, release
from evidence_io import sha, write_json


def fingerprint(obj, include_positions=True):
    h = hashlib.sha256()
    def add(value):
        h.update(np.asarray(value).tobytes())
    add([list(r) for r in obj.matrix_world])
    if include_positions:
        add([list(v.co) for v in obj.data.vertices])
    add([len(p.vertices) for p in obj.data.polygons])
    add([v for p in obj.data.polygons for v in p.vertices])
    add([p.material_index for p in obj.data.polygons])
    for uv in obj.data.uv_layers:
        h.update(uv.name.encode()); add([list(d.uv) for d in uv.data])
    for m in obj.data.materials:
        h.update(m.name.encode())
    return h.hexdigest()


def main():
    base = OUT/'restart3-tree06-root'
    dest = base/'depth-v1'
    dest.mkdir(exist_ok=False)
    probe = json.loads((base/'probe.json').read_text())
    source = Path(probe['model'])
    assert sha(source) == probe['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(source))
    bpy.context.view_layer.update()
    objects = [o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == 'croisement02-tree-06']
    original = {o.name: fingerprint(o) for o in objects}
    obj = next(o for o in objects if o.name.endswith('wood 057'))
    fixed = fingerprint(obj, False)
    old = np.array([tuple(obj.matrix_world @ v.co) for v in obj.data.vertices])
    projected = np.column_stack((old[:, 0], -SIN*old[:, 1]-COS*old[:, 2]))
    constraints = np.array([[r['pixel'][0]+.5, r['pixel'][1]+.5, r['required_native_ray_advance']+3] for r in probe['rays']])
    # The same source-plane field moves every depth layer. This preserves root
    # thickness along source rays instead of flattening the underside to ground.
    advance = np.zeros(len(old))
    for x, y, amount in constraints:
        distance = np.linalg.norm(projected-[x, y], axis=1)
        influence = np.where(distance < 40, .5*(1+np.cos(np.minimum(distance/40, 1)*np.pi)), 0)
        advance = np.maximum(advance, amount*influence)
    inverse = obj.matrix_world.inverted()
    for v, point, amount in zip(obj.data.vertices, old, advance):
        v.co = inverse @ (Vector(point)+RAY*float(amount))
    obj.data.update()
    bpy.context.view_layer.update()
    new = np.array([tuple(obj.matrix_world @ v.co) for v in obj.data.vertices])
    native_after = np.column_stack((new[:, 0], -SIN*new[:, 1]-COS*new[:, 2]))
    assert fingerprint(obj, False) == fixed
    assert all(fingerprint(o) == original[o.name] for o in objects if o != obj)
    obj.data.calc_loop_triangles()
    tree = BVHTree.FromPolygons(new.tolist(), [tuple(t.vertices) for t in obj.data.loop_triangles], all_triangles=True)
    checked = []
    for row in probe['rays']:
        x, y = row['pixel']
        origin = Vector((x+.5, -(y+.5)/SIN, 0))+RAY*5000
        hit, normal, face, distance = tree.ray_cast(origin, -RAY, 10000)
        assert hit is not None
        checked.append(dict(pixel=row['pixel'], hit=list(hit), bank=row['bank'], ray_clearance=float((hit-Vector(row['bank'])).dot(RAY))))
    bpy.context.preferences.filepaths.save_version = 0
    model = dest/'model.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(model))
    assert sha(source) == probe['model_sha256']
    write_json(dest/'report.json', dict(
        status='Private partial root-depth candidate; source appearance and contact review pending',
        input_model=str(source), input_sha256=sha(source), model_sha256=sha(model),
        changed_object=obj.name, unchanged_objects=[o.name for o in objects if o != obj],
        unchanged_topology_uv_material_transform=True, advance_max=float(advance.max()),
        source_projection_max_error=float(np.abs(native_after-projected).max()),
        source_constraints=checked, cleared_existing_rays=sum(r['ray_clearance']>.25 for r in checked),
        missing_contour_pixels=probe['missing'],
        limitations=['The 97 missing root-contour pixels are not repaired by depth movement.',
                     'Native first-hit RGBA, whole-domain ownership, bank intersection and actual eight-view review remain required.',
                     'This new geometry has no inherited approval.']))
    print(dest, flush=True)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
