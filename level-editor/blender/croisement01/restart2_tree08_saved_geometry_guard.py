"""Verify saved wood coordinates/topology against the fully audited loft candidate."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy
import numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
R=ROOT/'level-editor/work/active/croisement01/restart2';model=R/'tree08-wood-prototype-v23-smooth-join/model.blend';packet=R/'tree08-implicit-collar-v19-loft-seam/candidate.npz'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))>=6*1024**2
 digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');a=np.load(packet)
 local=np.array([v.co[:] for v in obj.data.vertices]);matrix=np.array(obj.matrix_world);world=local@matrix[:3,:3].T+matrix[:3,3];faces=np.array([p.vertices[:] for p in obj.data.polygons]);assert np.array_equal(world,a['vertices']) and np.array_equal(faces,a['faces']);assert obj.data.has_custom_normals
 normals=np.array([n.vector[:] for n in obj.data.corner_normals]);assert np.isfinite(normals).all() and (np.linalg.norm(normals,axis=1)>.999).all();assert sha(model)==digest
 report=dict(status='PASS_EXACT_SAVED_GEOMETRY',model_sha256=digest,candidate_sha256=sha(packet),vertices_exact=True,faces_exact=True,saved_custom_normals_valid=True)
 (model.parent/'saved-exact-geometry.json').write_text(json.dumps(report,indent=2)+'\n');print(report,flush=True)
finally:release()
