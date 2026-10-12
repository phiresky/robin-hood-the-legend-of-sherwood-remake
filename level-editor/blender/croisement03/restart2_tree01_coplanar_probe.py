"""Measure actual native alpha coverage of coincident observed layers; no model writes."""
import sys,json,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as common
import restart2_tree01_phase_geometry_guard as phase
B=common.B;F=B/'tree01-native-transition-saved-v1';O=F/'coplanar-native-probe-v1'
def main():
 assert not O.exists();common.resources();common.acquire(slots=2)
 try:
  common.resources();O.mkdir();model=F/'worker.blend';digest=phase.sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;names=['Tree01 observed native frame0 provisional cells','Tree01 observed native proposed static samples'];objects=[scene.objects[n]for n in names]
  for obj in scene.objects:
   if obj.type=='MESH':obj.hide_render=obj not in objects
  ray=Vector((0,-common.C,common.S));camera=bpy.data.objects.new('Exact native coverage camera',bpy.data.cameras.new('Exact native coverage camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=175;camera.data.clip_end=10000;camera.location=Vector((87.5,-87.5/common.S,0))+ray*5000;camera.rotation_euler=(-ray).to_track_quat('-Z','Y').to_euler();scene.camera=camera;scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.transparent_max_bounces=256;scene.cycles.use_denoising=False;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=175;scene.render.resolution_y=175;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.render.filter_size=.01
  sources=[np.array(Image.open(B/'tree01-native-fragment-v1/000.png').convert('RGBA')),np.array(Image.open(B/'tree01-static-leaf-proposal-v1/proposed-source-rgba.png').convert('RGBA'))];expected=(sources[0][:,:,3]>0)|(sources[1][:,:,3]>0);rows=[]
  for label,offset in [('coplanar',0),('static-recessed',.05)]:
   if offset:
    for v in objects[1].data.vertices:v.co-=ray*offset
    objects[1].data.update()
   scene.render.filepath=str(O/(label+'.png'));bpy.ops.render.render(write_still=True);rgba=np.array(Image.open(O/(label+'.png')).convert('RGBA'));covered=rgba[:,:,3]>127;row={'label':label,'static_recession_world':offset,'expected':int(expected.sum()),'covered':int(covered.sum()),'missing_known':int((expected&~covered).sum()),'extra':int((covered&~expected).sum())};rows.append(row)
  assert phase.sha(model)==digest;(O/'receipt.json').write_text(json.dumps({'status':'DIAGNOSTIC_ACTUAL_NATIVE_ALPHA','model_sha256':digest,'runs':rows,'limits':['Only observed native and static layers, exact pixel-center camera.','Second render changes static positions only in memory; no geometry saved.','No runtime or source ownership approval inferred.']},indent=2)+'\n');print(rows)
 finally:common.release()
if __name__=='__main__':main()
