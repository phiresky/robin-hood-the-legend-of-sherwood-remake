"""Render each imported full-path pose at matching source-camera scale."""
import sys,json,math,hashlib,shutil
from pathlib import Path
import bpy,bmesh
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
BASE=ROOT/'level-editor/work/croisement02-refinement';MODEL=BASE/'restart67-butterfly07-fullpath-export-v1/butterfly07-fullpath.glb';OUT=BASE/'restart69-butterfly07-local-review-v1'
def main():
 assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;OUT.mkdir();bpy.ops.wm.read_factory_settings(use_empty=True);s=bpy.context.scene;s.render.fps=25;bpy.ops.import_scene.gltf(filepath=str(MODEL));root=bpy.data.objects['Butterfly07 full inferred flight path'];owned=list(s.objects)
 s.render.engine='CYCLES';s.cycles.samples=4;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_x=s.render.resolution_y=128;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None'
 camera=bpy.data.cameras.new('Original art camera');camera.type='ORTHO';camera.ortho_scale=24;cam=bpy.data.objects.new(camera.name,camera);s.collection.objects.link(cam);s.camera=cam;direction=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))));records=[]
 for phase in range(99):
  s.frame_set(phase*2);center=root.matrix_world.translation;cam.location=center+direction*100;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();changed=[]
  for ob in owned:
   if ob.type!='MESH' or not ob.name.startswith('Rigid wing hinge'):continue
   original=ob.data;copy=original.copy();bm=bmesh.new();bm.from_mesh(copy);normal=ob.matrix_world.to_3x3().inverted().transposed();back=[f for f in bm.faces if (normal@f.normal).dot(cam.location-ob.matrix_world@f.calc_center_median())<=0];bmesh.ops.delete(bm,geom=back,context='FACES');bm.to_mesh(copy);bm.free();ob.data=copy;changed.append((ob,original,copy))
  s.render.filepath=str(OUT/f'{phase:03}.png');bpy.ops.render.render(write_still=True)
  for ob,original,copy in changed:ob.data=original;bpy.data.meshes.remove(copy)
  records.append(dict(phase=phase,frame=phase*2,camera_matrix_world=[list(r) for r in cam.matrix_world],ortho_scale=24))
 (OUT/'report.json').write_text(json.dumps(dict(model_sha256=hashlib.sha256(MODEL.read_bytes()).hexdigest(),views=records,policy='Temporary Cycles backface suppression matches one-sided glTF; model unchanged.'),indent=2)+'\n')
if __name__=='__main__':
 acquire(2)
 try:main()
 finally:release()
