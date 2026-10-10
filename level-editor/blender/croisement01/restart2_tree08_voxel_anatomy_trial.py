"""Private volumetric anatomy trial, with no claim of source/contact acceptance."""
import json
from pathlib import Path
import shutil
import sys
import bpy
import bmesh
import numpy as np

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from review_evidence import sha
from restart2_tree08_union import topology
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
source=R/'tree08-connected-tip-cleanup-v2/candidate.npz'
out=R/'tree08-volumetric-anatomy-v1'
assert not out.exists()
acquire(slots=2)
try:
    assert shutil.disk_usage(R).free>=10*1024**3
    assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
    data=np.load(source);v,f=data['vertices'],data['faces']
    origin=np.array([552.,-672.,235.])
    bpy.ops.wm.read_factory_settings(use_empty=True)
    mesh=bpy.data.meshes.new('Private volumetric wood')
    mesh.from_pydata((v-origin).tolist(),[],f.tolist());mesh.update()
    obj=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.objects.active=obj;obj.select_set(True)
    modifier=obj.modifiers.new('Half-unit volume reconstruction','REMESH')
    modifier.mode='VOXEL';modifier.voxel_size=.5;modifier.adaptivity=0
    print('VOLUME REMESH START',flush=True)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    smooth=obj.modifiers.new('Small-scale branch surface fairing','SMOOTH')
    smooth.factor=.5;smooth.iterations=12
    bpy.ops.object.modifier_apply(modifier=smooth.name)
    mesh=obj.data;mesh.update()
    normals=np.array([p.normal[:] for p in mesh.vertices])
    for vertex,normal in zip(mesh.vertices,normals):
        vertex.co+=__import__('mathutils').Vector(normal*.6)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update()
    vertices=np.array([p.co[:] for p in mesh.vertices])+origin
    faces=np.array([p.vertices[:] for p in mesh.polygons])
    check=topology(obj)
    out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=faces)
    (out/'report.json').write_text(json.dumps(dict(status='PRIVATE_ANATOMY_TRIAL_NOT_SOURCE_OR_CONTACT_VALIDATED',parent_sha256=sha(source),candidate_sha256=sha(out/'candidate.npz'),voxel_size=.5,smoothing_iterations=12,outward_normal_offset=.6,topology=check,scope='Reconstructed inferred wood volume. No source RGB, source coverage, root contact, strict intersections or approval claimed.'),indent=2)+'\n')
    print(check,flush=True)
finally:
    release()
