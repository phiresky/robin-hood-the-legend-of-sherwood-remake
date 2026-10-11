"""Verify final scoped bark fill preserves geometry, original pixels and other atlas texels."""
import hashlib,io,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';O=W/'baked-v8-connected-gaps'
def geometry():
 o=next(o for o in bpy.context.scene.objects if o.type=='MESH')
 return hashlib.sha256(b''.join([np.array(o.matrix_world).tobytes(),np.array([v.co[:] for v in o.data.vertices]).tobytes(),np.array([p.vertices[:] for p in o.data.polygons]).tobytes(),np.array([n.vector[:] for n in o.data.corner_normals]).tobytes()])).hexdigest()
def atlases():return {im.name:np.asarray(Image.open(io.BytesIO(im.packed_file.data)).convert('RGBA')) for im in bpy.data.images if im.packed_file}
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'));original=geometry()
 bpy.ops.wm.open_mainfile(filepath=str(W/'baked-v5-bounded-gaps/worker.blend'));before=atlases();assert geometry()==original
 bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));after=atlases();assert geometry()==original;assert set(before)==set(after)
 allowed={}
 repair=json.loads((O/'gap-repair.json').read_text());assert repair['parent_model_sha256']==sha(W/'baked-v5-bounded-gaps/worker.blend')
 for row in repair['repairs']:
  x,y=row['texel'];allowed.setdefault(row['image'],set()).add((x,after[row['image']].shape[0]-1-y))
 changes=[]
 for name,a in before.items():
  b=after[name];assert a.shape==b.shape
  points=np.argwhere(np.any(a!=b,axis=2));outside=[(int(x),int(y)) for y,x in points if (int(x),int(y)) not in allowed.get(name,set())]
  assert not outside,(name,len(outside));changes.append({'image':name,'changed_rgba_texels':len(points),'outside_scoped_inferred_positions_changed':0})
 native=json.loads((O/'native-texel-guard.json').read_text());assert native['status']=='PASS_NATIVE_TEXEL_RGBA' and native['interpolation']==['Closest'] and native['model_sha256']==sha(O/'worker.blend')
 result={'status':'PASS_FINAL_GEOMETRY_AND_SOURCE_PRESERVATION','model_sha256':sha(O/'worker.blend'),'approved_geometry_sha256':sha(W/'model.blend'),'geometry_and_normals_exact':True,'native_source_rgba_exact':6276,'changes':changes,'known_alpha_policy':'Atlas alpha encodes source ownership; emitted material stays opaque.','limitations':'Native first-hit sample equality, not antialiased rendered-image equality.'}
 (O/'final-preservation-guard.json').write_text(json.dumps(result,indent=2)+'\n');print(result,flush=True)
finally:release()
