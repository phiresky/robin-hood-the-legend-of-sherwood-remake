"""Repair inward face orientation without changing the approved jamb shape."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement/restart2'
OUT = WORK / 'jamb-orientation-v1'
SOURCE = WORK / 'gate-geometry-v10/covered/model.blend'
NAME = 'building-778-portcullis-jamb-return'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert sha(SOURCE) == '0acd1c449654078bf09684d558afba275053b5c24965e82fec1ad084a18bc116'
if OUT.exists():
    raise FileExistsError(OUT)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import bmesh
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from refinement_workspace import _geometry

bpy.ops.wm.open_mainfile(filepath=str(SOURCE))
bpy.context.view_layer.update()
obj = bpy.context.scene.objects[NAME]
outside = {o.name: _geometry(o, protect_appearance=True) for o in bpy.context.scene.objects if o != obj}
def shape():
    mesh = obj.data
    faces = []
    for p in mesh.polygons:
        corners = []
        for li in p.loop_indices:
            corners.append((mesh.loops[li].vertex_index, tuple(tuple(layer.data[li].uv) for layer in mesh.uv_layers)))
        faces.append((p.material_index, sorted(corners)))
    return {'vertices': [tuple(v.co) for v in mesh.vertices], 'faces': sorted(faces),
            'matrix': [list(r) for r in obj.matrix_world], 'materials': [m.name for m in mesh.materials]}
before = shape()
bm = bmesh.new()
bm.from_mesh(obj.data)
volume_before = bm.calc_volume(signed=True)
assert volume_before < 0, 'Expected inward closed shell; investigate instead of blindly flipping'
assert all(e.is_manifold for e in bm.edges)
bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
volume_after = bm.calc_volume(signed=True)
assert volume_after > 0
assert abs(volume_after + volume_before) < 1e-4
bm.to_mesh(obj.data)
bm.free()
obj.data.update()
assert shape() == before, 'Positions, topology, UV corners, materials or transform changed'
assert outside == {o.name: _geometry(o, protect_appearance=True) for o in bpy.context.scene.objects if o.name in outside}
OUT.mkdir(parents=True)
bpy.ops.wm.save_as_mainfile(filepath=str(OUT / 'model.blend'), compress=True)
(OUT / 'validation.json').write_text(json.dumps({
    'status': 'Technical orientation candidate; source-input review and root review pending',
    'source_model_sha256': sha(SOURCE), 'model_sha256': sha(OUT / 'model.blend'),
    'object': NAME, 'signed_volume_before': volume_before, 'signed_volume_after': volume_after,
    'exact_shape_uv_material_transform_preserved': True, 'outside_objects_exact': len(outside),
    'change': 'Face winding and corresponding normals only; no vertex or surface movement',
}, indent=2) + '\n')
print('JAMB ORIENTATION GUARDS PASS', flush=True)
