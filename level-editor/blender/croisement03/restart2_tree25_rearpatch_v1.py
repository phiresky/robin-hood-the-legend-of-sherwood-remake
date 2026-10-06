"""Test small planar rear leaf patches while retaining every native-facing face."""
import hashlib
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import bmesh
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'level-editor/refinement'), str(ROOT / 'level-editor/refinement/blender')]
from render_slots import acquire, release
from refinement_workspace import _geometry
from workspace_components import appearance_state

E = ROOT / 'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def uvkey(values):
    return tuple(sorted(tuple(round(c, 6) for c in uv) for uv in values))


def signature(face, uv, flags):
    return (face.material_index, tuple((tuple(v.co), tuple(tuple(loop[layer].uv) for layer in uv), tuple(loop[flags]))
        for v, loop in zip(face.verts, face.loops)))


def main():
    assert shutil.disk_usage(ROOT).free > 25 * 1024**3
    src = E / 'native-rgb-control-v1/worker.blend'
    assert sha(src) == '58d09f01e0cde42e7ca9b46d0a2f958d388bba23cf4b35f09a6d7bd1aec81802'
    out = E / 'rearpatch-geometry-v1'
    assert not out.exists()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(src))
        bpy.context.preferences.filepaths.save_version = 0
        obj = next(o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == 'croisement03-tree-25')
        outside = {o.name: _geometry(o, protect_appearance=True) for o in bpy.data.objects if o.type == 'MESH' and o != obj}
        materials = appearance_state(obj)['materials']
        matrix, inverse = obj.matrix_world.copy(), obj.matrix_world.inverted()
        bm = bmesh.new(); bm.from_mesh(obj.data)
        uv = bm.loops.layers.uv['Foliage UV']
        all_uv = list(bm.loops.layers.uv.values())
        flags = bm.loops.layers.float_color['Source ownership']
        fallback = bm.faces.layers.int['reprojection_fallback_material']
        old_bounds = [[min((matrix @ v.co)[i] for v in bm.verts), max((matrix @ v.co)[i] for v in bm.verts)] for i in range(3)]
        selected, front_by_face, preserved_backings = [], {}, 0
        for slot in (2, 6, 10):
            fronts = {uvkey(l[uv].uv for l in f.loops): f for f in bm.faces if f.material_index == slot - 1}
            assert len(fronts) == 128
            rear_count = 0
            for f in [f for f in bm.faces if f.material_index == slot]:
                known = fronts[uvkey(l[uv].uv for l in f.loops)]
                gap = (matrix @ f.calc_center_median() - matrix @ known.calc_center_median()).length
                if gap < .1:
                    assert .049 < gap < .051
                    preserved_backings += 1
                    continue
                assert gap > .34
                selected.append(f); front_by_face[f] = known; rear_count += 1
            assert rear_count == 128
        assert len(selected) == preserved_backings == 384
        selected_set = set(selected)
        immutable = [signature(f, all_uv, flags) for f in bm.faces if f not in selected_set]
        records, new_faces = [], []
        maximum_projection_error = 0.0
        for face in selected:
            corners = [matrix @ l.vert.co for l in face.loops]
            tex = [l[uv].uv.copy() for l in face.loops]
            all_tex = {layer: [l[layer].uv.copy() for l in face.loops] for layer in all_uv}
            known = front_by_face[face]
            lookup = {tuple(round(c, 6) for c in l[uv].uv): matrix @ l.vert.co for l in known.loops}
            fronts = [lookup[tuple(round(c, 6) for c in t)] for t in tex]
            # Four subdivisions per edge keep each patch at a small fraction of
            # the old lobe. Constant local Y prevents the curved dome from
            # elongating a few source leaf pixels across a large rear strip.
            n = 4
            bary_weights = lambda i, j: ((n-i-j)/n, i/n, j/n)
            for i in range(n):
                for j in range(n-i):
                    triangles = [(bary_weights(i,j), bary_weights(i+1,j), bary_weights(i,j+1))]
                    if i+j < n-1:
                        triangles.append((bary_weights(i+1,j), bary_weights(i+1,j+1), bary_weights(i,j+1)))
                    for ws in triangles:
                        points = [sum((p*w for p,w in zip(corners, weights)), Vector()) for weights in ws]
                        known_points = [sum((p*w for p,w in zip(fronts, weights)), Vector()) for weights in ws]
                        native_y = [-p.y*SIN-p.z*COS for p in points]
                        low = max(old_bounds[1][0], max(p.y+.1 for p in known_points),
                                  max((-s-old_bounds[2][1]*COS)/SIN for s in native_y))
                        high = min(old_bounds[1][1], min((-s-old_bounds[2][0]*COS)/SIN for s in native_y))
                        assert low <= high
                        plane_y = min(high, max(low, sum(p.y for p in points)/3))
                        projected = [Vector((p.x, plane_y, (-s-plane_y*SIN)/COS)) for p,s in zip(points,native_y)]
                        projection_error = max(abs(-p.y*SIN-p.z*COS-s) for p,s in zip(projected,native_y))
                        maximum_projection_error = max(maximum_projection_error, projection_error)
                        assert projection_error < .0002
                        vertices = [bm.verts.new(inverse @ p) for p in projected]
                        new = bm.faces.new(vertices); new.material_index = face.material_index
                        new[fallback] = face[fallback]
                        for loop, weights in zip(new.loops, ws):
                            for layer, coordinates in all_tex.items():
                                loop[layer].uv = sum((p*w for p,w in zip(coordinates, weights)), Vector((0,0)))
                            loop[flags] = (0,1,1,1)
                        new_faces.append(new)
            records.append(dict(slot=face.material_index, old_area=face.calc_area()))
        bmesh.ops.delete(bm, geom=selected, context='FACES')
        new_set = set(new_faces)
        assert [signature(f, all_uv, flags) for f in bm.faces if f not in new_set] == immutable
        assert len(new_faces) == 6144
        bm.normal_update(); bm.to_mesh(obj.data); bm.free(); obj.data.update()
        new_bounds = [[min((matrix @ v.co)[i] for v in obj.data.vertices), max((matrix @ v.co)[i] for v in obj.data.vertices)] for i in range(3)]
        assert max(abs(a-b) for before,after in zip(old_bounds,new_bounds) for a,b in zip(before,after)) < .0002
        assert appearance_state(obj)['materials'] == materials
        assert outside == {o.name: _geometry(o, protect_appearance=True) for o in bpy.data.objects if o.name in outside}
        out.mkdir()
        bpy.ops.wm.save_as_mainfile(filepath=str(out / 'worker.blend'))
        report = dict(status='Private geometry candidate; native/all-eight/contact review required',
            source_model_sha256=sha(src), model_sha256=sha(out / 'worker.blend'),
            changed_rear_slots=[2,6,10], removed_faces=384, new_faces=6144,
            preserved_native_front_backing_faces=preserved_backings, all_other_faces_exact=True,
            all_image_and_material_data_unchanged=True, outside_objects_unchanged=len(outside),
            before_bounds=old_bounds, after_bounds=new_bounds,
            projected_native_triangle_footprints_preserved=True,
            maximum_projection_error=maximum_projection_error, projection_error_limit=.0002,
            limitations=['New inferred rear coverage requires fresh geometry approval.',
                        'Own-source RGB remains diagnostic only; no appearance approval implied.',
                        'Actual native first-hit and wall contacts still require checks.'])
        (out / 'construction.json').write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(report))
    finally:
        release()


if __name__ == '__main__':
    main()
