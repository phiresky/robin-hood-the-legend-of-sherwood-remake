"""Private two-boulder completion of the ambiguous initial native cap."""
import sys,json
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import sha,point
from tree_geometry import RAY,SIN,COS
from render_slots import acquire,release


def main():
 base=OUT/'rock-trap-state-candidate-v11';dest=OUT/'rock-trap-state-candidate-v13';dest.mkdir(exist_ok=True);assert not(dest/'worker.blend').exists()
 report=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==report['model_sha256'];acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;bpy.context.view_layer.update()
  left,right,foot=[bpy.data.objects[f'covered inferred complete boulder {i:02}']for i in range(3)]
  guard={o.name:[tuple(o.matrix_world@v.co)for v in o.data.vertices]for o in scene.objects if o.type=='MESH'and not o.get('state_endpoint')=='covered'}
  bm=bmesh.new()
  for o in [right,foot]:
   for v in o.data.vertices:bm.verts.new(right.matrix_world.inverted()@o.matrix_world@v.co)
  result=bmesh.ops.convex_hull(bm,input=list(bm.verts),use_existing_faces=False)
  discard=list(set(g for g in result['geom_interior']+result['geom_unused']if isinstance(g,bmesh.types.BMVert)))
  if discard:bmesh.ops.delete(bm,geom=discard,context='VERTS')
  bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(right.data);bm.free();right.data.update();bpy.data.objects.remove(foot,do_unlink=True)
  # The contact indentation is the exact adjacent complete solid, not a cap slice.
  bpy.context.view_layer.objects.active=left;mod=left.modifiers.new('Exact adjacent boulder contact','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=right;bpy.ops.object.modifier_apply(modifier=mod.name)
  source=json.loads((OUT/'state-target-evidence/rock-trap/manifest.json').read_text());x,y,r,b=source['bbox'];records=[]
  for obj in [left,right]:
   bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.to_mesh(obj.data);bm.free();obj.data.update()
   if not obj.data.uv_layers:obj.data.uv_layers.new(name='Native target projection')
   for face in obj.data.polygons:
    face.material_index=0 if face.normal.dot(RAY)>.05 else 1
    for loop in face.loop_indices:
     p=obj.matrix_world@obj.data.vertices[obj.data.loops[loop].vertex_index].co;obj.data.uv_layers.active.data[loop].uv=((p.x-x)/(r-x),1-(-p.y*SIN-p.z*COS-y)/(b-y))
   records.append(dict(object=obj.name,vertices=len(obj.data.vertices),faces=len(obj.data.polygons),closed=True,volume=volume))
  for name,vertices in guard.items():assert vertices==[tuple(bpy.data.objects[name].matrix_world@v.co)for v in bpy.data.objects[name].data.vertices]
  bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));report.update(model_sha256=sha(dest/'worker.blend'),base_model_sha256=sha(base/'worker.blend'),status='private two-body hypothesis; source and contact review pending',covered_body_hypothesis='Two complete boulders; lower-right native cap is treated as the same body as upper-right cap. Initial count and correspondence to five applied fragments are unproven.',initial_pair=records)
  (dest/'manifest.json').write_text(json.dumps(report,indent=2)+'\n')
  target=point((x+r)/2,(y+b)/2,0);scene.camera.location=target+RAY*3000;scene.camera.rotation_euler=(target-scene.camera.location).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(dest/'covered-source-actual-bank1.png');bpy.ops.render.render(write_still=True)
 finally:release()
if __name__=='__main__':main()
