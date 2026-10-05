"""Read-only native lower31 joint with its exact saved foliage at converged budgets."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY
from render_slots import acquire,release

def main():
 worker=tree_workspace(31);digest=sha(worker/'model.blend');out=OUT/'restart2-wood/tree31-native-joint-v1';out.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name];transforms={o:o.matrix_world.copy() for o in objects};scene=bpy.data.scenes.new('Native31 wood and exact saved foliage');bpy.context.window.scene=scene
 for original in objects:
  obj=original.copy();obj.parent=None;obj.matrix_world=transforms[original];obj.hide_render=False;scene.collection.objects.link(obj)
 box=[890,540,1060,700];left,top,right,bottom=box;w=right-left;h=bottom-top;target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Native31 exact');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=w;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera;scene.world=bpy.data.worlds.new('Black emission environment');scene.world.color=(0,0,0);scene.render.engine='CYCLES';scene.cycles.samples=16;scene.render.resolution_x=w;scene.render.resolution_y=h;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
 arrays=[]
 for budget in [256,512]:
  scene.cycles.transparent_max_bounces=budget;scene.render.filepath=str(out/f'native-{budget}.png');bpy.ops.render.render(write_still=True,scene=scene.name);arrays.append(np.array(Image.open(out/f'native-{budget}.png').convert('RGBA')))
 source=Image.open(worker/'reference/source.png').convert('RGB').crop(box);display=Image.new('RGBA',(w,h),(100,100,100,255));display.alpha_composite(Image.fromarray(arrays[-1]));sheet=Image.new('RGB',(w*2,h));sheet.paste(source,(0,0));sheet.paste(display.convert('RGB'),(w,0));sheet.resize((w*8,h*4),Image.Resampling.NEAREST).save(out/'source-comparison.png');diff=np.abs(arrays[1].astype(int)-arrays[0].astype(int));write_json(out/'evidence.json',dict(model_sha256=digest,worker=str(worker),objects=[o.name for o in objects],source_crop=box,budgets=[256,512],convergence_max_channel_difference=int(diff.max()),convergence_changed_pixels=int(np.any(diff,axis=2).sum()),scope='Exact saved31 wood and crown local native view. Ground and adjacent independent plants excluded; unknown regions not assigned semantic ownership.',source_comparison_sha256=sha(out/'source-comparison.png')))
 if sha(worker/'model.blend')!=digest:raise ValueError('Read-only input changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
