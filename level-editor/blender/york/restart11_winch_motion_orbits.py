"""Render complete motion poses at the frozen camera aspect, retaining failed crops."""
import json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-motion-physical-v2';OUT=BASE/'complete-orbits-v2'
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
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;motion=json.loads((BASE/'motion.json').read_text());OUT.mkdir()
for index in (0,22,36,44):
 scene.frame_set(motion['rows'][index]['tick']);bpy.context.view_layer.update()
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004'
 out=OUT/f'pose-{index:02d}';out.mkdir();names={f'view-{i}':f'Winch{i}' for i in range(8)};rows=[]
 for i in range(8):
  cam=scene.objects[f'Winch{i}'];rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cam.matrix_world],'ortho_scale':cam.data.ortho_scale})
 (out/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(out/'views.json');scene.render.resolution_x=320;scene.render.resolution_y=384;render_views(scene.name,names,out/'renders',modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
 for i in range(8):
  im=Image.open(out/f'renders/view-{i}-textured.png');assert im.size==(320,384);sheet.paste(im,((i%4)*320,(i//4)*384))
 sheet.save(out/'solid8.png');labeled_copy(out/'solid8.png',out/'solid8-native-labeled.png')
print('COMPLETE MOTION ORBITS SAVED')
