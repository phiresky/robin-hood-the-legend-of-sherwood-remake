"""Join a private reconstructed upper collar to the exact retained lower root mesh."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Matrix
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from restart2_tree08_union import topology
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
smooth_transition='--smooth-transition' in sys.argv
O=R/('tree08-implicit-collar-v11-smooth-stitched' if smooth_transition else 'tree08-implicit-collar-v3-stitched');assert not O.exists()
upper_path=R/('tree08-implicit-collar-v10-smooth-source' if smooth_transition else 'tree08-implicit-collar-v2-source')/'candidate.npz';lower_path=R/'tree08-connected-tip-cleanup-v2/candidate.npz'
assert upper_path.exists() and lower_path.exists()
origin=np.array([552.,-672.,235.]);s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray=np.array([0.,-c,s]);down=np.array([0.,-s,-c])
def mesh_object(name,vertices,faces):
 mesh=bpy.data.meshes.new(name);mesh.from_pydata((vertices-origin).tolist(),[],faces.tolist());mesh.update()
 obj=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(obj);obj.matrix_world=Matrix.Translation(origin);return obj
def clean(obj):
 bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=1e-5);bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(obj.data);bm.free();obj.data.update()
def cube(name,cut,upper):
 center=np.array([552.,0.,0.])+ray*700+down*(cut+(-1000 if upper else 1000));v=np.array([center+np.array([x*1000,0,0])+ray*y*1000+down*z*1000 for x,y,z in [(-1,-1,-1),(-1,-1,1),(-1,1,-1),(-1,1,1),(1,-1,-1),(1,-1,1),(1,1,-1),(1,1,1)]])
 return mesh_object(name,v,np.array([[0,2,3,1],[4,5,7,6],[0,1,5,4],[2,6,7,3],[0,4,6,2],[1,3,7,5]])[:,::-1])
def boolean(obj,other,operation):
 bpy.context.view_layer.objects.active=obj;obj.select_set(True);mod=obj.modifiers.new(operation,'BOOLEAN');mod.operation=operation;mod.solver='EXACT';mod.object=other
 bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(other,do_unlink=True);clean(obj)
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.read_factory_settings(use_empty=True)
 a=np.load(upper_path);upper=mesh_object('Reconstructed upper wood',a['vertices'],a['faces']);clean(upper)
 a=np.load(lower_path);lower=mesh_object('Retained exact lower roots',a['vertices'],a['faces'])
 boolean(upper,cube('Upper clip',325,True),'INTERSECT');print('UPPER CLIPPED',flush=True)
 boolean(lower,cube('Lower clip',315,False),'INTERSECT');print('LOWER CLIPPED',flush=True)
 boolean(lower,upper,'UNION');print('STITCH UNION DONE',flush=True)
 check=topology(lower);v=np.array([p.co[:] for p in lower.data.vertices])+origin;f=np.array([p.vertices[:] for p in lower.data.polygons])
 O.mkdir();np.savez_compressed(O/'candidate.npz',vertices=v,faces=f)
 report=dict(status='PRIVATE_STITCH_REQUIRES_CONTACT_SOURCE_AND_STRICT_REVIEW',candidate_sha256=hashlib.sha256((O/'candidate.npz').read_bytes()).hexdigest(),upper_sha256=hashlib.sha256(upper_path.read_bytes()).hexdigest(),lower_sha256=hashlib.sha256(lower_path.read_bytes()).hexdigest(),topology=check,overlap_native_y=[315,325],source='Upper implicit reconstruction; retained lower original geometry. No source coverage, exact saved color, contact, or full strict claim until independent checks.')
 (O/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
finally:release()
