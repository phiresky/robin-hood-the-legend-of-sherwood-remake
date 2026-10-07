"""Inspect the full inferred chain return and its native endpoint before phase fitting."""
import json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-chain-loop-prototype-v3';OUT=BASE/'review'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0,str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest,labeled_copy
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.frame_set(88);bpy.context.view_layer.update();OUT.mkdir();names={f'view-{i}':f'Winch{i}' for i in range(8)};rows=[]
for i in range(8):
 cam=scene.objects[f'Winch{i}'];rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cam.matrix_world],'ortho_scale':cam.data.ortho_scale})
(OUT/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(OUT/'views.json')
for o in scene.objects:
 if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004'
scene.render.resolution_x=320;scene.render.resolution_y=384;render_views(scene.name,names,OUT/'renders',modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
for i in range(8):
 im=Image.open(OUT/f'renders/view-{i}-textured.png');assert im.size==(320,384);sheet.paste(im,((i%4)*320,(i//4)*384))
sheet.save(OUT/'solid8.png');labeled_copy(OUT/'solid8.png',OUT/'solid8-native-labeled.png')
