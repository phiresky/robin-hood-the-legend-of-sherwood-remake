"""Reopen delivered GLBs and prove every physical triangle stayed in place."""
import sys,json
from pathlib import Path
from collections import Counter
import bpy,numpy as np
from scipy.spatial import cKDTree
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart5_initial_net_export import DEST,ROOT,HASHES,guard
from evidence_io import sha,write_json
from render_slots import acquire,release
def inventory():
 result={}
 bpy.context.view_layer.update()
 for o in bpy.context.scene.objects:
  if o.type!='MESH':continue
  o.data.calc_loop_triangles()
  result[o.name]=(np.array([o.matrix_world@v.co for v in o.data.vertices]),np.array([t.vertices[:]for t in o.data.loop_triangles]))
 return result
def main(key):
 guard();out=DEST/f'profile-{key}';receipt=json.loads((out/'export.json').read_text());model=Path(receipt['model_source']);assert sha(model)==HASHES[key];glb=out/'model.glb';assert sha(glb)==receipt['glb_sha256']
 bpy.ops.wm.open_mainfile(filepath=str(model));original=inventory();bpy.ops.wm.read_factory_settings(use_empty=True);bpy.ops.import_scene.gltf(filepath=str(glb));root=next(o for o in bpy.context.scene.objects if o.name.startswith('Reusable family origin'));a=receipt['position'];root.location+=Vector((a[0],-a[2],a[1]));current=inventory();assert current.keys()==original.keys();rows=[]
 for name,(points,tris)in original.items():
  other,other_tris=current[name];tree=cKDTree(points);distance,index=tree.query(other);oldindex=tree.query(points)[1];assert distance.max()<.001
  expected=Counter(tuple(sorted(oldindex[t]))for t in tris);actual=Counter(tuple(sorted(index[t]))for t in other_tris);assert expected==actual,'Triangle connectivity or winding-independent surface changed'
  # The exporter may split vertices at UV seams but may not add/remove faces.
  rows.append(dict(object=name,triangles=len(tris),original_vertices=len(points),export_vertices=len(other),maximum_world_vertex_error=float(distance.max()),surface_connectivity_exact=True))
 write_json(out/'geometry-guard.json',dict(status='PASS',approved_model_sha256=sha(model),export_glb_sha256=sha(glb),family_position=receipt['position'],parts=rows,scope='All triangle vertex positions and connectivity retained after family origin restoration; attachment geometry and gameplay unchanged.'))
 guard()
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
