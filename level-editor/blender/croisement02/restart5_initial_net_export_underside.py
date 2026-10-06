"""Review exported hidden surfaces using the approved native-first cameras."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Matrix,Vector
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
from restart5_initial_net_export import DEST,guard
from evidence_io import sha,write_json
from render_slots import acquire,release
def main(key):
 guard();out=DEST/f'profile-{key}';receipt=json.loads((out/'export.json').read_text());model=Path(receipt['model_source']);glb=out/'model.glb';assert sha(model)==receipt['model_sha256'];assert sha(glb)==receipt['glb_sha256'];parent=model.parent;views=json.loads((parent/'underside-v2/evidence.json').read_text())['views'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
 for o in list(scene.objects):
  if o.type=='MESH':bpy.data.objects.remove(o,do_unlink=True)
 bpy.ops.import_scene.gltf(filepath=str(glb));root=next(o for o in scene.objects if o.name.startswith('Reusable family origin'));a=receipt['position'];root.location+=Vector((a[0],-a[2],a[1]));bpy.context.view_layer.update();scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.render.film_transparent=True;scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';folder=out/'underside';folder.mkdir(exist_ok=False);sheet=Image.new('RGBA',(1024,1024));rows=[]
 for i,v in enumerate(views):
  data=bpy.data.cameras.new('Approved underside view');data.type='ORTHO';data.ortho_scale=v['ortho_scale'];data.clip_start=.1;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.matrix_world=Matrix(v['camera']);scene.camera=camera;scene.render.filepath=str(folder/f'view-{i}.png');bpy.ops.render.render(write_still=True);image=Image.open(folder/f'view-{i}.png').convert('RGBA');sheet.paste(image,((i%2)*512,(i//2)*512));p=np.array(Image.open(parent/f'underside-v2/view-{i}.png').convert('RGBA')).astype(int);q=np.array(image).astype(int);mask=(p[:,:,3]>0)|(q[:,:,3]>0);d=abs(p[:,:,:3]-q[:,:,:3]);rows.append(dict(view=i,alpha_changed=int((p[:,:,3]!=q[:,:,3]).sum()),mean_rgb=float(d[mask].mean()),max_rgb=int(d[mask].max())));bpy.data.objects.remove(camera,do_unlink=True)
 sheet.save(folder/'sheet.png');write_json(folder/'evidence.json',dict(export_glb_sha256=sha(glb),approved_model_sha256=sha(model),approved_camera_receipt_sha256=sha(parent/'underside-v2/evidence.json'),native_first=True,views=rows,new_aggregate_bytes=guard()))
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
