"""Native-first full saved-asset review for the final continuous fork."""
import sys,json
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT,OUT,SPECS,RAY,SIN
from restart4_stump_final_contact import frame,sheet
from render_multiview_asset import render
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 out=ROOT/'tree24-fork-union-v10';model=out/'model.blend';digest=sha(model);guard=json.loads((out/'fork-guard.json').read_text());nativeguard=json.loads((out/'retained-surface-native-guard.json').read_text());assert guard['model_sha256']==nativeguard['model_sha256']==digest;assert guard['crown_exact']and guard['original_packed_images_exact']and not guard['lost_native'];assert not nativeguard['missing']and all(r['exact']for r in nativeguard['native_edge_samples']);bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';base=json.loads((SPECS[24][0].parent/'input/views.json').read_text());base['object_names']=[o.name for o in objects];base.pop('render_object_names',None)
 for i,v in enumerate(base['views']):
  direction=RAY if i==0 else Matrix(v['camera_matrix_world']).to_quaternion()@Vector((0,0,1));cam=frame(scene,objects,direction,320,1.22);v['camera_matrix_world']=[list(r)for r in cam.matrix_world];v['ortho_scale']=cam.data.ortho_scale;v['crop']={'width':320,'height':320}
 write_json(out/'cameras.json',base);render(out/'cameras.json',out/'actual',modes=('textured','solid'),width=320)
 for mode in ['textured','solid']:sheet([out/f'actual/view-{i}-{mode}.png'for i in range(8)],out/f'{mode}-sheet.png')
 for o in objects:o.hide_render='Crown'in o.name
 cx,cy,scale=25,845,110;center=Vector((cx,-cy/SIN,0));camera=bpy.data.objects.new('Original native fork camera',bpy.data.cameras.new('Original native fork camera'));scene.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.ortho_scale=scale;camera.data.clip_end=20000;camera.location=center+RAY*5000;camera.rotation_euler=(center-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera;scene.render.resolution_x=512;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.filepath=str(out/'native-wood.png');bpy.ops.render.render(write_still=True);box=[cx-scale/2,cy-scale/2,cx+scale/2,cy+scale/2];native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').transform((512,512),Image.Transform.EXTENT,box,Image.Resampling.NEAREST);native.save(out/'native-source.png');pic=Image.open(out/'native-wood.png').convert('RGBA');comparison=Image.new('RGBA',(1536,512),(35,35,35,255));comparison.paste(native,(0,0));comparison.paste(Image.alpha_composite(native,pic),(512,0));comparison.paste(pic,(1024,0));comparison.save(out/'source-comparison.png');write_json(out/'native-camera.json',dict(box=box,camera=[list(r)for r in camera.matrix_world],scale=scale));assert sha(model)==digest
finally:release()
