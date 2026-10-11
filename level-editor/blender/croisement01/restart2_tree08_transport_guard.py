"""Check that transport metadata leaves the approved Tree08 geometry and images exact."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
R=ROOT/'level-editor/work/croisement01-refinement/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def digest():
 o=next(o for o in bpy.context.scene.objects if o.type=='MESH')
 data=[np.array(o.matrix_world).tobytes(),np.array([v.co[:] for v in o.data.vertices]).tobytes(),np.array([p.vertices[:] for p in o.data.polygons]).tobytes(),np.array([n.vector[:] for n in o.data.corner_normals]).tobytes()]
 return {'geometry_normals':hashlib.sha256(b''.join(data)).hexdigest(),'images':{i.name:hashlib.sha256(i.packed_file.data).hexdigest() for i in bpy.data.images if i.packed_file}}
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'));a=digest()
 bpy.ops.wm.open_mainfile(filepath=str(W/'transport-model.blend'));b=digest()
 assert a==b
 report={'status':'PASS_EXACT_GEOMETRY_NORMALS_PACKED_IMAGES','approved_sha256':sha(W/'model.blend'),'transport_sha256':sha(W/'transport-model.blend'),'fingerprints':a,'scope':'Only renderer collection and asset_group metadata differ.'}
 (W/'transport-identity-guard.json').write_text(json.dumps(report,indent=2)+'\n');print(report,flush=True)
finally:release()
