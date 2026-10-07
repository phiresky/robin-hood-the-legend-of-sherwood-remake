"""Extract only pinned support surfaces and verify evaluated native ray hits."""
import io
import json
import shutil
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from tree_geometry import RAY,SIN
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
DEST=OUT/'restart14-hidden-archer/surface-v8'
NAMES=['Northwest Rock Outcrop / Northwest Rock Outcrop part 035','North Woodland Bank / North Woodland Bank part 000']
def budget(size=0):
 assert shutil.disk_usage(OUT).free-size>=10*1024**3
 assert sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file())+size<=8*1024**2

def main():
 budget(8*1024**2);assert not DEST.exists()
 audit=OUT/'restart14-hidden-archer/audit-v1/substrate-first-hit-v1/report.json';r=json.loads(audit.read_text());source=Path(r['source']);assert sha(source)==r['source_sha256']
 bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
 with bpy.data.libraries.load(str(source),link=True) as (src,dst):
  assert all(n in src.objects for n in NAMES),[(n,[q for q in src.objects if n.split(' / ')[-1] in q]) for n in NAMES]
  dst.objects=NAMES
 objects=list(dst.objects)
 for obj in objects:
  parent=obj
  while parent:
   if parent.name not in scene.objects:scene.collection.objects.link(parent)
   parent=parent.parent
 bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();arrays={};records=[]
 for k,obj in enumerate(objects):
  evaluated=obj.evaluated_get(deps);mesh=evaluated.to_mesh();mesh.calc_loop_triangles();vertices=np.array([evaluated.matrix_world@v.co for v in mesh.vertices],dtype=np.float64);triangles=np.array([t.vertices[:] for t in mesh.loop_triangles],dtype=np.int32)
  arrays[f'vertices{k}']=vertices;arrays[f'triangles{k}']=triangles
  records.append(dict(name=obj.name,vertices=len(vertices),triangles=len(triangles),matrix_world=[list(row) for row in obj.matrix_world],evaluated_matrix_world=[list(row) for row in evaluated.matrix_world],local_min=np.min([v.co[:] for v in mesh.vertices],axis=0).tolist(),world_min=vertices.min(0).tolist(),world_max=vertices.max(0).tolist(),parent_chain=[]))
  p=obj.parent
  while p:records[-1]['parent_chain'].append(dict(name=p.name,matrix_world=[list(row) for row in p.matrix_world]));p=p.parent
  evaluated.to_mesh_clear()
 tree,owners,_=_tree(objects);samples=next(p['samples'] for p in r['profiles'] if p['profile'].endswith('05'));miss=[];maxerr=0
 for s in samples:
  x,y=s['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000;p,_,i,_=tree.ray_cast(origin,-RAY)
  error=float(np.linalg.norm(np.array(p)-s['world'])) if p is not None else None
  if error is not None:maxerr=max(maxerr,error)
  if p is None or owners[i].name!=s['object'] or error>.02:miss.append(dict(pixel=s['pixel'],expected=s['object'],actual=owners[i].name if p is not None else None,error=error))
 stream=io.BytesIO();np.savez_compressed(stream,**arrays);payload=stream.getvalue();budget(len(payload)+65536);DEST.mkdir();(DEST/'surfaces.npz').write_bytes(payload)
 write_json(DEST/'extraction.json',dict(status='PASS' if not miss else 'HOLD',source=str(source),source_sha256=sha(source),point_authority_sha256=sha(audit),surfaces_sha256=sha(DEST/'surfaces.npz'),records=records,native_rechecks=len(samples),max_world_error=maxerr,mismatches=miss,loaded_scene_meshes=[o.name for o in scene.objects if o.type=='MESH'],limits=['Native sample agreement verifies evaluated placement only, not closed-volume support or geodesic clearance.','Saved triangles include material-transparent surfaces; opaque acceptance is separately tested against original shaders.']))
 assert not miss,miss[:3]
 print('PASS',len(samples),maxerr,len(payload),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
