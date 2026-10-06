"""Separate complementary frontend cutouts along the source ray."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from workspace_components import appearance_state
from refinement_workspace import _geometry

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()

def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
 source=e/'cluster-geometry-v2';out=e/'cluster-geometry-v3'
 assert not out.exists();assert shutil.disk_usage(ROOT).free>25*1024**3
 report=json.loads((source/'construction.json').read_text());assert sha(source/'worker.blend')==report['model_sha256'];acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source/'worker.blend'));bpy.context.preferences.filepaths.save_version=0
  o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');m=o.data
  before=appearance_state(o);outside={x.name:_geometry(x,protect_appearance=True) for x in bpy.data.objects if x.type=='MESH' and x!=o}
  slot=next(i for i,mat in enumerate(m.materials) if mat.name=='Tree25 small clusters / inferred front')
  selected={v for f in m.polygons if f.material_index==slot for v in f.vertices};assert selected
  assert not selected&{v for f in m.polygons if f.material_index!=slot for v in f.vertices}
  positions={v.index:tuple(v.co) for v in m.vertices if v.index not in selected}
  delta=Vector((0,.02,-.02*math.tan(math.radians(35))));local=o.matrix_world.inverted().to_3x3()@delta
  for index in selected:m.vertices[index].co+=local
  m.update();assert positions=={v.index:tuple(v.co) for v in m.vertices if v.index not in selected}
  assert appearance_state(o)==before
  assert outside=={x.name:_geometry(x,protect_appearance=True) for x in bpy.data.objects if x.name in outside}
  out.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
  shutil.copyfile(source/'native-samples.npz',out/'native-samples.npz')
  report.update(model_sha256=sha(out/'worker.blend'),previous_cluster_model_sha256=sha(source/'worker.blend'),
   inferred_front_separation_world=list(delta),moved_inferred_front_vertices=len(selected),
   reason='Alpha-complementary frontend faces were coplanar. Move inferred frontend behind known frontend and ahead of backing, keeping native projection unchanged.')
  (out/'construction.json').write_text(json.dumps(report,indent=2)+'\n')
  print(dict(model_sha256=report['model_sha256'],vertices=len(selected)))
 finally:release()
if __name__=='__main__':main()
