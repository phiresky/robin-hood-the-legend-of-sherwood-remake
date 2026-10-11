"""Restore the source-bearing tip connection lost during volume sampling."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from scipy.spatial import cKDTree
from mathutils import Matrix,Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from restart2_tree08_union import topology
R=ROOT/'level-editor/work/croisement01-refinement/restart2';packet=R/'tree08-implicit-collar-v3-stitched';out=R/'tree08-implicit-collar-v4-connected';assert not out.exists()
a=np.load(packet/'candidate.npz');v,f=a['vertices'],a['faces'];labels=np.load(packet/'component-labels.npz');report=json.loads((packet/'components-source.json').read_text());assert [x['source_pixels'] for x in report['components']]==[6253,0,23]
origin=np.array([552.,-672.,235.])
def mesh_object(name,component):
 faces=f[labels['faces']==component];used,remap=np.unique(faces,return_inverse=True);mesh=bpy.data.meshes.new(name);mesh.from_pydata((v[used]-origin).tolist(),[],remap.reshape(-1,3).tolist());mesh.update();obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj);obj.matrix_world=Matrix.Translation(origin);return obj
def union(a,b):
 bpy.context.view_layer.objects.active=a;a.select_set(True);modifier=a.modifiers.new('Inferred tip connection','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.object=b;bpy.ops.object.modifier_apply(modifier=modifier.name);bpy.data.objects.remove(b,do_unlink=True)
 bm=bmesh.new();bm.from_mesh(a.data);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=1e-7);bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(a.data);bm.free();a.data.update()
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.read_factory_settings(use_empty=True);main=mesh_object('Retained connected upper and lower wood',0);tip=mesh_object('Source-bearing terminal twig',2)
 vv=v[labels['vertices']==0];tt=v[labels['vertices']==2];dist,index=cKDTree(vv).query(tt);j=dist.argmin();p,q=vv[index[j]],tt[j];direction=(q-p)/np.linalg.norm(q-p);p=p-direction*1.5;q=q+direction*1.5
 bpy.ops.mesh.primitive_cylinder_add(vertices=16,radius=1.4,depth=float(np.linalg.norm(q-p)),location=(p+q)/2)
 bridge=bpy.context.object;bridge.rotation_euler=Vector(direction).to_track_quat('Z','Y').to_euler();bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
 union(main,bridge);print('BRIDGE JOINED',flush=True);union(main,tip);print('TIP JOINED',flush=True)
 vertices=np.array([p.co[:] for p in main.data.vertices])+origin;faces=np.array([p.vertices[:] for p in main.data.polygons]);check=topology(main)
 out.mkdir();np.savez_compressed(out/'candidate.npz',vertices=vertices,faces=faces)
 result=dict(status='PRIVATE_TIP_CONNECTED_RECHECK_REQUIRED',source_sha256=hashlib.sha256((packet/'candidate.npz').read_bytes()).hexdigest(),candidate_sha256=hashlib.sha256((out/'candidate.npz').read_bytes()).hexdigest(),topology=check,excluded_component=dict(id=1,faces=8,source_pixels=0),tip_source_pixels=23,inferred_connector=dict(radius=1.4,original_gap=float(dist[j]),main_endpoint=p.tolist(),tip_endpoint=q.tolist()),limits='No texture synthesis or acceptance. Source coverage, contact, strict intersections and final anatomy remain to be checked.')
 (out/'report.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2),flush=True)
finally:release()
