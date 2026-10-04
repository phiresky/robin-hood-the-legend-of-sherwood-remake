"""Reconstruct the narrow market house's lower body, jetty and thick roof.

Source measurements live in the separate geometry pass. Hidden rear walls and
the foundation footprint are inferred; this private candidate is not approved.
"""
import hashlib
import json
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'level-editor/work/york-refinement'
ASSET = 'york-market-southeast-tall-narrow-house'
WORK = OUT / 'geometry-pass-01/assets' / ASSET


def main():
    sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
    from render_slots import acquire
    acquire()
    sys.path.insert(0, str(ROOT / 'level-editor/blender/nottingham'))
    from freeze_tooling import select_tooling
    select_tooling(json.loads((OUT / 'tooling/current.json').read_text())['directory'])
    import bpy
    import bmesh
    from mathutils import Matrix
    from refinement_workspace import modified, validate
    from evidence_io import record_recipe
    bpy.ops.wm.open_mainfile(filepath=str(WORK / 'model.blend'))
    scene = bpy.data.scenes['york Refinement']
    bpy.context.window.scene = scene
    target = [o for o in bpy.data.collections['york Working'].all_objects
              if o.type == 'MESH' and o.get('asset_group') == ASSET and not o.hide_render]
    if len(target) != 1 or target[0]['source_node'] != 'building-312':
        raise ValueError('Unexpected narrow-house ownership')
    obj = target[0]
    if obj.get('york_geometry_candidate'):
        raise ValueError('Refuse to apply reconstruction twice')
    materials = list(obj.data.materials)
    vertices, faces = [], []
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))

    def ring(points, heights):
        start = len(vertices)
        for (x, y), z in zip(points, heights):
            vertices.append((x, -y / sine, z / cosine))
        return list(range(start, start + len(points)))

    def join(a, b):
        for i in range(len(a)):
            k = (i + 1) % len(a)
            faces.append((a[i], a[k], b[k], b[i]))

    floor = [(584, 1473), (623, 1492.5), (644, 1477), (605, 1457.5)]
    upper = [(576, 1469.5), (623, 1492.5), (644, 1477), (597, 1454)]
    a = ring(floor, [90.00101] * 4)
    b = ring(floor, [129.5] * 4)
    c = ring(upper, [129.5] * 4)
    d = ring(upper, [197.5, 197.5, 220.5, 220.5])
    faces.append(tuple(reversed(a)))
    join(a, b)
    join(b, c)
    join(c, d)
    faces.append(tuple(d))
    roof = [(573, 1468), (622, 1492), (644, 1477), (595, 1453)]
    e = ring(roof, [197.5, 197.5, 220.5, 220.5])
    f = ring(roof, [200, 200, 223, 223])
    faces.append(tuple(reversed(e)))
    join(e, f)
    faces.append(tuple(f))
    mesh = bpy.data.meshes.new('York narrow house complete candidate')
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    bm = bmesh.new()
    bm.from_mesh(mesh)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=0.00001)
    bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=0.00001)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    if any(not edge.is_manifold for edge in bm.edges):
        raise ValueError('Reconstruction contains an open/nonmanifold edge')
    bm.to_mesh(mesh)
    bm.free()
    old = obj.data
    obj.data = mesh
    obj.parent = None
    obj.matrix_world = Matrix.Identity(4)
    for material in materials:
        mesh.materials.append(material)
    mesh.uv_layers.new(name='UVMap')
    obj['york_geometry_candidate'] = 'narrow-house-01'
    obj['inferred_surfaces'] = 'Lower footprint, rear body and roof continuation beneath adjacent roof'
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(WORK / 'model.blend'))
    record_recipe(WORK, __file__)
    modified(WORK)
    result = validate(WORK)
    (WORK / 'geometry-notes.json').write_text(json.dumps({
        'status': 'private candidate; source coverage and terrain joint review pending',
        'source_observations': str(OUT / 'geometry-pass-01/narrow-house-observations.json'),
        'topology': 'Closed lower/upper body and closed thick roof; roof-body overlap is intentional construction.',
        'validation': result,
        'limitations': ['Fixed baseline cameras may clip newly reconstructed lower body; full-object supplementary views required.',
                       'Authored full-house source domain and adjacent-roof ownership need review.',
                       'Native terrain contact must be checked in a joint render.',
                       'No geometry approval or texture generation authorization inferred.']}, indent=2) + '\n')


if __name__ == '__main__':
    main()
