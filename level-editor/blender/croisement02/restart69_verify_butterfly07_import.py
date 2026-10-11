"""Verify the actual Blender importer at the core clip's zero-based frame origin."""
import bpy,json,hashlib
from pathlib import Path
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/croisement02-refinement';MODEL=BASE/'restart67-butterfly07-fullpath-export-v1/butterfly07-fullpath.glb';REF=BASE/'restart67-butterfly07-browser-request-v1/reference.json';OUT=BASE/'restart67-butterfly07-fullpath-export-v1/import-verification.json'
def main():
 assert not OUT.exists();bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(MODEL));rows=json.loads(REF.read_text())['rows'];maximum=0;count=0
 for tick in range(0,198,2):
  scene.frame_set(tick);expected=rows[tick]
  for side in range(2):
   ob=bpy.data.objects[f'Rigid wing hinge {side}'];world=[ob.matrix_world@v.co for v in ob.data.vertices]
   for p in expected['wings'][side]:
    wanted=Vector((p[0],-p[2],p[1]));error=min((wanted-actual).length for actual in world);maximum=max(maximum,error);assert error<.001,(tick,side,error);count+=1
 result=dict(status='PASS_ACTUAL_IMPORT_ZERO_BASED_POSES',model_sha256=hashlib.sha256(MODEL.read_bytes()).hexdigest(),reference_sha256=hashlib.sha256(REF.read_bytes()).hexdigest(),source_poses=99,world_vertices_verified=count,maximum_world_distance=maximum,frame_origin=0,frame_step=2);OUT.write_text(json.dumps(result,indent=2)+'\n');print(result)
if __name__=='__main__':main()
