"""Verify actual observed coverage for every exact native phase; no model writes."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as common
import restart2_tree01_phase_geometry_guard as phase
B=common.B;F=B/'tree01-native-transition-saved-v2';O=F/'all-phase-native-coverage-v1'
def main():
 assert not O.exists();common.resources();common.acquire(slots=2)
 try:
  common.resources();O.mkdir();model=F/'worker.blend';digest=phase.sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;names=['Tree01 observed native frame0 provisional cells','Tree01 observed native proposed static samples'];objects=[scene.objects[n]for n in names]
  for obj in scene.objects:
   if obj.type=='MESH':obj.hide_render=obj not in objects
  ray=Vector((0,-common.C,common.S));camera=bpy.data.objects.new('Exact native coverage camera',bpy.data.cameras.new('Exact native coverage camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=175;camera.data.clip_end=10000;camera.location=Vector((87.5,-87.5/common.S,0))+ray*5000;camera.rotation_euler=(-ray).to_track_quat('-Z','Y').to_euler();scene.camera=camera;scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.transparent_max_bounces=256;scene.cycles.use_denoising=False;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=175;scene.render.resolution_y=175;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.render.filter_size=.01
  static=np.array(Image.open(B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png').convert('RGBA'));rows=[];tex=next(n for n in objects[0].data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');original=tex.image
  for phase_index in range(14):
   path=B/'tree01-native-fragment-v1'/f'{phase_index:03}.png';src=np.array(Image.open(path).convert('RGBA'));expected=(src[:,:,3]>0)|(static[:,:,3]>0);image=bpy.data.images.load(str(path),check_existing=False);image.pack();tex.image=image;label=f'phase-{phase_index:02}';scene.render.filepath=str(O/(label+'.png'));bpy.ops.render.render(write_still=True);rgba=np.array(Image.open(O/(label+'.png')).convert('RGBA'));covered=rgba[:,:,3]>127;rows.append({'phase':phase_index,'source_sha256':phase.sha(path),'expected':int(expected.sum()),'covered':int(covered.sum()),'missing_known':int((expected&~covered).sum()),'extra':int((covered&~expected).sum())});tex.image=original;bpy.data.images.remove(image)
  assert phase.sha(model)==digest;good=all(r['missing_known']==0 and r['extra']==0 for r in rows);report={'status':'PASS_ACTUAL_ALL_PHASE_OBSERVED_COVERAGE'if good else'HOLD_ACTUAL_SOURCE_COVERAGE','model_sha256':digest,'runs':rows,'limits':['Only observed native and static layers, exact pixel-center camera; complete-tree occlusion checked separately.','Exact native phases injected in memory; runtime clock and single owner not proven.','No saved geometry or source ownership changes.']};(O/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');print(report['status'],rows);assert good

 finally:common.release()
if __name__=='__main__':main()
