"""Remove an internal applied-boulder overlap while preserving the visible solid union."""
import sys,json
from pathlib import Path
import bpy,bmesh
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import sha
from tree_geometry import RAY,SIN,COS
from render_slots import acquire,release


def main():
 base=OUT/'rock-trap-state-candidate-v13';dest=OUT/'rock-trap-state-candidate-v14';dest.mkdir(exist_ok=True);assert not(dest/'worker.blend').exists();report=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==report['model_sha256'];acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;obj=bpy.data.objects['applied inferred complete boulder 02'];other=bpy.data.objects['applied inferred complete boulder 03'];guard={o.name:([tuple(v.co)for v in o.data.vertices],[tuple(p.vertices)for p in o.data.polygons],[tuple(row)for row in o.matrix_world])for o in scene.objects if o.type=='MESH'and o!=obj}
  bpy.context.view_layer.objects.active=obj;mod=obj.modifiers.new('Adjacent endpoint contact','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=other;bpy.ops.object.modifier_apply(modifier=mod.name)
  bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.to_mesh(obj.data);bm.free();obj.data.update()
  x,y,r,b=json.loads((OUT/'state-target-evidence/rock-trap/manifest.json').read_text())['bbox']
  for face in obj.data.polygons:
   face.material_index=0 if face.normal.dot(RAY)>.05 else 1
   for loop in face.loop_indices:
    p=obj.matrix_world@obj.data.vertices[obj.data.loops[loop].vertex_index].co;obj.data.uv_layers.active.data[loop].uv=((p.x-x)/(r-x),1-(-p.y*SIN-p.z*COS-y)/(b-y))
  assert guard=={o.name:([tuple(v.co)for v in o.data.vertices],[tuple(p.vertices)for p in o.data.polygons],[tuple(row)for row in o.matrix_world])for o in scene.objects if o.type=='MESH'and o!=obj}
  bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));report.update(model_sha256=sha(dest/'worker.blend'),base_model_sha256=sha(base/'worker.blend'),status='Private endpoint contact correction; reopened verification pending',applied_contact_correction=dict(object=obj.name,adjacent=other.name,method='Exact volume difference removes interior overlap; combined visible union is unchanged.',closed=True,remaining_volume=volume,other_geometry_and_transforms_unchanged=True))
  (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
