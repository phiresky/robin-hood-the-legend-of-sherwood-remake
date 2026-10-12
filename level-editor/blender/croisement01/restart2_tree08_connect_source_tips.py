"""Private exact union trial for touching inferred branch shells."""
import json
from pathlib import Path
import shutil
import sys
import bpy
import bmesh
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'level-editor/refinement'), str(Path(__file__).parent)]
from render_slots import acquire, release
from restart2_tree08_union import topology
from review_evidence import sha

R = ROOT/'level-editor/work/active/croisement01/restart2'
source = R/'tree08-unified-source-fairing-cpu-v1/candidate.npz'
out = R/'tree08-connected-tip-union-v1'
assert not out.exists()
acquire(slots=2)
try:
    assert shutil.disk_usage(R).free >= 10*1024**3
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:')) >= 6*1024**2
    out.mkdir()
    data = np.load(source)
    v, f = data['vertices'], data['faces']
    parent = np.arange(len(v))
    def root(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    for face in f:
        a = root(face[0])
        for b in face[1:]:
            parent[root(b)] = a
    labels = np.array([root(i) for i in range(len(v))])
    groups = [np.flatnonzero(labels[f[:,0]] == k) for k in np.unique(labels)]
    groups.sort(key=len, reverse=True)
    assert [len(g) for g in groups] == [147544,252,188,22]
    groups = groups[:3]
    plan = json.loads((source.parent/'inferred-connections-plan.json').read_text())
    assert sha(source) == plan['input_sha256']
    bpy.ops.wm.read_factory_settings(use_empty=True)
    objects = []
    origin = np.array([552., -672., 235.])
    for i, ids in enumerate(groups):
        used, remap = np.unique(f[ids], return_inverse=True)
        mesh = bpy.data.meshes.new(f'shell-{i}')
        mesh.from_pydata((v[used]-origin).tolist(), [], remap.reshape(-1,3).tolist())
        mesh.update()
        obj = bpy.data.objects.new(mesh.name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        objects.append(obj)
    from mathutils import Vector
    for number, connection in enumerate(plan['connections']):
        a=np.array(connection['position']);b=np.array(connection['nearest_main_position'])
        direction=(b-a)/np.linalg.norm(b-a)
        center=(a+b)/2-origin
        bpy.ops.mesh.primitive_cylinder_add(vertices=24, radius=connection['inferred_connection_radius'], depth=float(np.linalg.norm(b-a)+2*connection['endpoint_overlap']), end_fill_type='NGON', location=center)
        twig=bpy.context.object
        twig.name=f'Inferred tip connection {number}'
        twig.rotation_euler=Vector(direction).to_track_quat('Z','Y').to_euler()
        bpy.ops.object.transform_apply(location=False,rotation=True,scale=True)
        objects.append(twig)
    base = objects[0]
    operands = bpy.data.collections.new('Inferred branch shell operands')
    bpy.context.scene.collection.children.link(operands)
    for obj in objects[1:]:
        operands.objects.link(obj)
    bpy.context.view_layer.objects.active = base
    modifier = base.modifiers.new('Join intersecting branch shells', 'BOOLEAN')
    modifier.operation = 'UNION'
    modifier.solver = 'EXACT'
    modifier.operand_type = 'COLLECTION'
    modifier.collection = operands
    modifier.use_self = True
    print('UNION', [len(g) for g in groups], flush=True)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    bm = bmesh.new()
    bm.from_mesh(base.data)
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(base.data)
    bm.free()
    base.data.update()
    vertices = np.array([p.co[:] for p in base.data.vertices])+origin
    faces = np.array([p.vertices[:] for p in base.data.polygons])
    check = topology(base)
    np.savez_compressed(out/'candidate.npz', vertices=vertices, faces=faces)
    (out/'report.json').write_text(json.dumps(dict(status='UNION_TRIAL_PENDING_SOURCE_AND_INTERSECTION_GUARDS', source_sha256=sha(source), candidate_sha256=sha(out/'candidate.npz'), original_shell_faces=[len(g) for g in groups], topology=check), indent=2)+'\n')
    print(check, flush=True)
finally:
    release()
