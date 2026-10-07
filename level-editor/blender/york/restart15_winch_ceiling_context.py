"""Show exact clipped ceiling triangles at the inferred winch hanger contacts.

Run with --check-only for a read-only geometric diagnostic. The clipped pieces
are review context, never replacement ceiling geometry or saved model edits.
"""
import hashlib
import json
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
WORK = ROOT / 'level-editor/work/york-refinement'
BASE = WORK / 'restart2/winch-supported-hardware-v1'
OUT = BASE / 'ceiling-context-v1'
CHECK = '--check-only' in sys.argv


def budget():
    used = sum(p.stat().st_size for b in (BASE, WORK / 'restart2/winch-guided-entry-candidate-v1')
               for p in b.rglob('*') if p.is_file())
    assert used < 20 * 1024**2, used
    assert shutil.disk_usage(ROOT).free > 10 * 1024**3 + 20 * 1024**2 - used


def clip(poly, axis, bound, sign):
    result = []
    for a, b in zip(poly, poly[1:] + poly[:1]):
        da, db = sign * (a[axis] - bound), sign * (b[axis] - bound)
        if da <= 0:
            result.append(a)
        if (da <= 0) != (db <= 0):
            result.append(a + (b - a) * (da / (da - db)))
    return result


if not CHECK:
    budget()
    if OUT.exists():
        raise FileExistsError(OUT)
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

bpy.ops.wm.open_mainfile(filepath=str(BASE / 'model.blend'))
scene = bpy.context.scene
scene.frame_set(0)
bpy.context.view_layer.update()
proposal = json.loads((BASE / 'proposal.json').read_text())
assert hashlib.sha256((BASE / 'model.blend').read_bytes()).hexdigest() == proposal['model_sha256']
records, context = [], []
for anchor in proposal['ceiling_anchors']:
    owner = scene.objects[anchor['owner']]
    owner.data.calc_loop_triangles()
    point = Vector(anchor['point_world'])
    vertices, faces, source_faces = [], [], []
    for triangle in owner.data.loop_triangles:
        poly = [owner.matrix_world @ owner.data.vertices[i].co for i in triangle.vertices]
        normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalized()
        if normal.z > -0.5:
            continue
        for axis, radius in enumerate((1.25, 1.25, 2.5)):
            poly = clip(poly, axis, point[axis] + radius, 1)
            poly = clip(poly, axis, point[axis] - radius, -1)
        if len(poly) < 3:
            continue
        offset = len(vertices)
        vertices.extend(poly)
        faces.append(tuple(range(offset, offset + len(poly))))
        source_faces.append(triangle.polygon_index)
    assert vertices, anchor
    tree = BVHTree.FromPolygons(vertices, faces)
    nearest = tree.find_nearest(point)
    assert nearest[0] is not None and nearest[3] < 0.001, nearest
    mesh = bpy.data.meshes.new(f'Exact clipped ceiling {anchor["hanger"]}')
    mesh.from_pydata(vertices, [], faces)
    obj = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(obj)
    obj.color = (0.65, 0.56, 0.40, 1)
    context.append(obj)
    records.append({'owner': owner.name, 'source_node': owner.get('source_node'),
                    'source_polygon_indices': sorted(set(source_faces)),
                    'anchor_world': list(point), 'anchor_surface_distance': nearest[3],
                    'clip_half_extents_world': [1.25, 1.25, 2.5],
                    'vertices_world': [list(v) for v in vertices], 'faces': faces})
report = {'model_sha256': proposal['model_sha256'], 'frame': 0,
          'scope': 'Exact cropped underside triangles from saved private room; context only, room not approved',
          'ceiling_fragments': records, 'geometry_changed': False}
if CHECK:
    print(json.dumps(report))
    sys.exit(0)

from PIL import Image
sys.path.insert(0, str(ROOT / 'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0, str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest, labeled_copy
budget()
OUT.mkdir()
own = [o for o in scene.objects if o.type == 'MESH' and o.get('native_patch') == 'patch-004']
for obj in scene.objects:
    if obj.type == 'MESH':
        obj.hide_render = obj not in own + context
targets = context + [o for o in own if min((o.matrix_world @ v.co).z for v in o.data.vertices) > 225
                     or o.name.startswith(('Travelling round', 'Inferred traveller'))]
points = [o.matrix_world @ v.co for o in targets for v in o.data.vertices]
center = (Vector(tuple(min(p[i] for p in points) for i in range(3))) +
          Vector(tuple(max(p[i] for p in points) for i in range(3)))) / 2
rows = []
scene.render.threads_mode = 'FIXED'
scene.render.threads = 2
for i in range(8):
    camera = scene.objects[f'Winch{i}']
    camera.location = center + (camera.matrix_world.to_quaternion() @ Vector((0, 0, 1))) * 10000
    bpy.context.view_layer.update()
    inv = camera.matrix_world.to_quaternion().inverted()
    projected = [inv @ (p - center) for p in points]
    camera.data.ortho_scale = max(max(p.y for p in projected) - min(p.y for p in projected),
                                 (max(p.x for p in projected) - min(p.x for p in projected)) * 384 / 320) * 1.25
    rows.append({'index': i, 'azimuth_degrees': i * 45,
                 'camera_matrix_world': [list(row) for row in camera.matrix_world],
                 'ortho_scale': camera.data.ortho_scale})
    budget()
    scene.render.resolution_x, scene.render.resolution_y = 320, 384
    render_views(scene.name, {f'view-{i}': camera.name}, OUT / f'renders/view-{i}', modes=('solid',), width=320)
budget()
(OUT / 'views.json').write_text(json.dumps({'layout': {'columns': 4, 'rows': 2}, 'views': rows}, indent=2) + '\n')
audit_manifest(OUT / 'views.json')
sheet = Image.new('RGBA', (1280, 768))
for i in range(8):
    sheet.paste(Image.open(OUT / f'renders/view-{i}/view-{i}-solid.png'), ((i % 4) * 320, (i // 4) * 384))
budget()
sheet.save(OUT / 'solid8.png')
budget()
labeled_copy(OUT / 'solid8.png', OUT / 'solid8-native-labeled.png')
budget()
(OUT / 'context.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({'output': str(OUT), 'status': 'Context rendered; no model edited or saved'}))
