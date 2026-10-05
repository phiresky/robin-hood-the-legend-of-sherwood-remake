"""Compare the complete saved wood projection with its own native source crop."""
import argparse,json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY

def main():
 p=argparse.ArgumentParser();p.add_argument('--workspace',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.workspace.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False);digest=sha(w/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update()
 objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==w.name and o.get('projection_component')!='crown'];points=np.asarray([o.matrix_world@v.co for o in objects for v in o.data.vertices]);screen=np.column_stack([points[:,0],-points[:,1]*SIN-points[:,2]*COS]);lo=np.floor(screen.min(0)-6).astype(int);hi=np.ceil(screen.max(0)+6).astype(int);lo=np.maximum(lo,[0,0]);hi=np.minimum(hi,[1792,1152]);left,top=lo;right,bottom=hi;width,height=hi-lo
 scene=bpy.data.scenes.new('Own native continuous wood');bpy.context.window.scene=scene
 for original in objects:
  obj=original.copy();obj.parent=None;obj.matrix_world=original.matrix_world.copy();obj.hide_render=False;scene.collection.objects.link(obj)
 scene.world=bpy.data.worlds.new('Unlit source projection');scene.world.color=(0,0,0);scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.transparent_max_bounces=256
 target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));data=bpy.data.cameras.new('Native camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=float(width);data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
 scene.render.resolution_x=int(width);scene.render.resolution_y=int(height);scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.filepath=str(out/'native.png');bpy.ops.render.render(write_still=True,scene=scene.name)
 actual=Image.open(out/'native.png').convert('RGBA');source=Image.open(w/'reference/source.png').convert('RGB').crop((int(left),int(top),int(right),int(bottom)));display=Image.new('RGBA',actual.size,(90,90,90,255));display.alpha_composite(actual);sheet=Image.new('RGB',(int(width)*2,int(height)));sheet.paste(source,(0,0));sheet.paste(display.convert('RGB'),(int(width),0));sheet.resize((int(width)*6,int(height)*3),Image.Resampling.NEAREST).save(out/'source-comparison.png');write_json(out/'evidence.json',dict(model=str(w/'model.blend'),model_sha256=digest,source_sha256=sha(w/'reference/source.png'),crop=[int(v) for v in [left,top,right,bottom]],comparison_sha256=sha(out/'source-comparison.png'),scope='Own wood only. Foliage and ground intentionally excluded; neutral unknown surfaces retain no invented source color.'))
 if sha(w/'model.blend')!=digest:raise ValueError('Model changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
